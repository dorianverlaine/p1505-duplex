//! HTTP side, served behind nginx at /duplex/:
//!
//! - `/`: the "continue" page (HTML, plain forms, reloads itself);
//! - `/api/...`: JSON for the macOS menu bar app;
//! - `/api/events`: server-sent events with a fresh snapshot on every change.
//!
//! Runs as p1505-duplex (group lpadmin) so it may act on anyone's job. Each
//! request gets its own thread, so event streams do not block the page.

use crate::actions::{Actions, Side, UserError};
use crate::config::Config;
use crate::ipp::Cups;
use crate::state::Store;
use crate::status::{DuplexJob, JobStateText, Reader, Snapshot, Stage};
use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server};

/// How often an event stream re-reads the state, and how long it may stay
/// silent before sending a keep-alive comment.
const POLL: Duration = Duration::from_secs(2);
const KEEPALIVE: Duration = Duration::from_secs(15);

pub fn main() -> Result<()> {
    let cfg = Config::load()?;
    let server =
        Server::http(&cfg.listen).map_err(|e| anyhow!("listening on {}: {e}", cfg.listen))?;
    eprintln!("listening on {}", cfg.listen);
    let app = Arc::new(App {
        cups: Cups::local()?,
        store: Store::new(&cfg.state_dir),
        cfg,
    });
    for req in server.incoming_requests() {
        let app = Arc::clone(&app);
        std::thread::spawn(move || app.serve(req));
    }
    Ok(())
}

type Resp = Response<std::io::Cursor<Vec<u8>>>;

struct App {
    cfg: Config,
    cups: Cups,
    store: Store,
}

#[derive(Deserialize)]
struct ReprintBody {
    side: Side,
    first: u32,
    last: u32,
}

#[derive(Deserialize)]
struct TestPageBody {
    user: String,
}

impl App {
    fn reader(&self) -> Reader<'_> {
        Reader {
            cfg: &self.cfg,
            cups: &self.cups,
            store: &self.store,
        }
    }

    fn actions(&self) -> Actions<'_> {
        Actions {
            cfg: &self.cfg,
            cups: &self.cups,
            store: &self.store,
        }
    }

    fn serve(&self, mut req: Request) {
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("/").to_string();
        let method = req.method().clone();
        if method == Method::Get && path == "/api/events" {
            let writer = req.into_writer();
            self.events(writer);
            return;
        }
        let mut body = String::new();
        let _ = req.as_reader().take(64 * 1024).read_to_string(&mut body);
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let resp = if parts.first() == Some(&"api") {
            self.api(&method, &parts[1..], &body)
        } else {
            self.page(&method, &parts, &body)
        };
        let _ = req.respond(resp);
    }

    // ---- HTML page -------------------------------------------------------

    fn page(&self, method: &Method, parts: &[&str], body: &str) -> Resp {
        let result = match (method, parts) {
            (Method::Get, [""] | ["index.html"]) => Ok(html(200, index(&self.reader().snapshot()))),
            (Method::Post, ["jobs", id, action]) => self
                .form_action(id, action, &form(body))
                .map(|()| redirect("../../")),
            _ => {
                return html(
                    404,
                    page(
                        "找不到頁面",
                        "<p>找不到頁面。</p><p><a href=\"./\">回到列表</a></p>",
                    ),
                );
            }
        };
        result.unwrap_or_else(|e| {
            eprintln!("{method} {}: {e:#}", parts.join("/"));
            let body = format!(
                "<div class=\"card\"><p class=\"bad\">{}</p><p><a href=\"{}\">回到列表</a></p></div>",
                esc(&message(&e)),
                if parts.len() == 3 { "../../" } else { "./" },
            );
            html(500, page("發生錯誤", &body))
        })
    }

    fn form_action(&self, id: &str, action: &str, form: &HashMap<String, String>) -> Result<()> {
        let id = parse_id(id)?;
        let a = self.actions();
        match action {
            "continue" => a.continue_job(id),
            "remove" => a.remove(id),
            "reprint" => {
                let side = match form.get("side").map(String::as_str) {
                    Some("front") => Side::Front,
                    Some("back") => Side::Back,
                    _ => return Err(bad("不明的列印面")),
                };
                a.reprint(id, side, num(form, "first")?, num(form, "last")?)
            }
            _ => Err(bad("不明的動作")),
        }
    }

    // ---- JSON API --------------------------------------------------------

    fn api(&self, method: &Method, parts: &[&str], body: &str) -> Resp {
        let a = self.actions();
        let result: Result<serde_json::Value> = match (method, parts) {
            (Method::Get, ["status"]) => {
                serde_json::to_value(self.reader().snapshot()).map_err(Into::into)
            }
            (Method::Get, ["printer"]) => {
                serde_json::to_value(self.reader().details()).map_err(Into::into)
            }
            (Method::Get, ["version"]) => Ok(serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "api": 1,
            })),
            (Method::Post, ["jobs", id, "continue"]) => {
                parse_id(id).and_then(|id| a.continue_job(id)).map(ok)
            }
            (Method::Post, ["jobs", id, "remove"]) => {
                parse_id(id).and_then(|id| a.remove(id)).map(ok)
            }
            (Method::Post, ["jobs", id, "reprint"]) => parse_id(id).and_then(|id| {
                let b: ReprintBody = json_body(body)?;
                a.reprint(id, b.side, b.first, b.last).map(ok)
            }),
            (Method::Post, ["queue", id, "cancel"]) => {
                parse_id(id).and_then(|id| a.cancel_queue_job(id)).map(ok)
            }
            (Method::Post, ["printer", "pause"]) => a.pause_printer().map(ok),
            (Method::Post, ["printer", "resume"]) => a.resume_printer().map(ok),
            (Method::Post, ["test-page"]) => json_body::<TestPageBody>(body)
                .and_then(|b| a.test_page(&b.user))
                .map(|job| serde_json::json!({ "ok": true, "job": job })),
            _ => Err(UserError {
                message: "找不到這個 API".into(),
                not_found: true,
            }
            .into()),
        };
        match result {
            Ok(v) => json(200, &v),
            Err(e) => {
                let code = match e.downcast_ref::<UserError>() {
                    Some(u) if u.not_found => 404,
                    Some(_) => 400,
                    None => 500,
                };
                if code == 500 {
                    eprintln!("{method} /api/{}: {e:#}", parts.join("/"));
                }
                json(code, &serde_json::json!({ "error": message(&e) }))
            }
        }
    }

    /// Stream a snapshot whenever it changes, until the client goes away.
    fn events(&self, mut out: Box<dyn Write + Send>) {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\n\
                    Cache-Control: no-store\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n\
                    retry: 3000\n\n";
        if out
            .write_all(head.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            return;
        }
        let mut last = String::new();
        let mut quiet_since = Instant::now();
        loop {
            let snap: Snapshot = self.reader().snapshot();
            let data = serde_json::to_string(&snap).unwrap_or_default();
            let msg = if data != last {
                last = data;
                quiet_since = Instant::now();
                Some(format!("event: status\ndata: {last}\n\n"))
            } else if quiet_since.elapsed() >= KEEPALIVE {
                quiet_since = Instant::now();
                Some(": keep-alive\n\n".to_string())
            } else {
                None
            };
            if let Some(msg) = msg
                && out
                    .write_all(msg.as_bytes())
                    .and_then(|()| out.flush())
                    .is_err()
            {
                return;
            }
            std::thread::sleep(POLL);
        }
    }
}

fn ok(_: ()) -> serde_json::Value {
    serde_json::json!({ "ok": true })
}

fn bad(message: &str) -> anyhow::Error {
    UserError {
        message: message.into(),
        not_found: false,
    }
    .into()
}

fn parse_id(id: &str) -> Result<i32> {
    id.parse().map_err(|_| {
        UserError {
            message: "找不到這份工作".into(),
            not_found: true,
        }
        .into()
    })
}

fn json_body<T: serde::de::DeserializeOwned>(body: &str) -> Result<T> {
    serde_json::from_str(body).map_err(|e| bad(&format!("請求格式錯誤：{e}")))
}

/// User errors as written; anything else with its full chain.
fn message(e: &anyhow::Error) -> String {
    match e.downcast_ref::<UserError>() {
        Some(u) => u.message.clone(),
        None => format!("{e:#}"),
    }
}

fn json(code: u16, value: &serde_json::Value) -> Resp {
    Response::from_data(serde_json::to_vec(value).unwrap_or_default())
        .with_status_code(code)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
}

// ---- HTML rendering ------------------------------------------------------

fn index(snap: &Snapshot) -> String {
    let mut out = String::new();
    match (&snap.printer, &snap.cups_error) {
        (Some(p), _) if !p.reasons.is_empty() || p.state == "stopped" => {
            let mut items: Vec<String> = p.reasons.iter().map(|r| r.text.clone()).collect();
            if !p.message.is_empty() {
                items.push(p.message.clone());
            }
            if items.is_empty() {
                items.push("印表機已停止".into());
            }
            let class = if p.blocking { "bad" } else { "warn" };
            let list: String = items
                .iter()
                .map(|i| format!("<li>{}</li>", esc(i)))
                .collect();
            let _ = write!(
                out,
                "<div class=\"banner {class}\"><strong>印表機狀態</strong><ul>{list}</ul></div>"
            );
        }
        (None, Some(_)) => {
            out += "<div class=\"banner bad\">連不上 CUPS，無法取得印表機狀態。</div>"
        }
        _ => {}
    }
    if snap.jobs.is_empty() {
        out += "<div class=\"empty\">沒有進行中的雙面列印工作</div>";
    }
    for job in &snap.jobs {
        out += &card(job);
    }
    page("P1505 手動雙面", &out)
}

fn side_text(side: &Option<crate::status::Side>) -> &'static str {
    side.as_ref().map_or("狀態不明", |s| s.state.text())
}

fn card(job: &DuplexJob) -> String {
    let sheets = job.sheets;
    let mut c = String::new();
    let _ = write!(
        c,
        "<div class=\"card\"><div class=\"t\">{}</div>\
         <div class=\"m\">{} 頁 · {} 張紙 · {} · {}</div>\
         <dl><dt>正面</dt><dd>{}</dd><dt>背面</dt><dd>{}</dd></dl>",
        esc(&job.title),
        job.pages,
        sheets,
        esc(&job.user),
        local_time(job.created),
        side_text(&job.front),
        side_text(&job.back),
    );

    let base = format!("jobs/{}/", job.id);
    let continue_button = |enabled: bool| {
        format!(
            "<form method=\"post\" action=\"{base}continue\"><button class=\"go\"{}>繼續列印背面</button></form>",
            if enabled { "" } else { " disabled" }
        )
    };
    match job.stage {
        Stage::ReadyToFlip => {
            let _ = write!(
                c,
                "<p class=\"hint\">正面已印完：取出這 {sheets} 張紙，放回紙匣後按「繼續」。</p>"
            );
            c += &continue_button(true);
        }
        Stage::FrontPrinting => {
            c += "<p class=\"hint\">正面還在列印，印完後再繼續。</p>";
            c += &continue_button(false);
        }
        Stage::Blocked => {
            c += "<p class=\"hint bad\">印表機目前無法列印，排除問題後再繼續。</p>";
            c += &continue_button(false);
        }
        Stage::FrontFailed => {
            c += "<p class=\"hint bad\">正面沒有印完。請用下方「出問題了？」補印正面，再繼續。</p>";
            c += &continue_button(false);
        }
        Stage::Done => {
            c += "<p class=\"hint ok\">雙面列印完成。</p>";
            let _ = write!(
                c,
                "<form method=\"post\" action=\"{base}remove\"><button class=\"go\">完成，移除這份工作</button></form>"
            );
        }
        Stage::BackFailed => {
            c += "<p class=\"hint bad\">背面沒有印完。如果有紙卡住或一次進了兩張，用下方「出問題了？」補印。</p>";
        }
        Stage::BackPrinting | Stage::Unknown => {}
    }

    let _ = write!(
        c,
        "<details><summary>出問題了？</summary>\
         <p class=\"note\">卡紙、一次進了兩張紙，或某幾張印壞了，都可以只補印那幾張。紙張依正面出紙順序編號，第 1 張的正面是第 1 頁。</p>\
         {}{}\
         <form method=\"post\" action=\"{base}remove\" onsubmit=\"return confirm('取消還沒印的部分並移除這份工作？')\">\
         <button class=\"x\">取消並移除這份工作</button></form></details></div>",
        reprint_form(&base, "front", "重印正面", "", sheets),
        reprint_form(
            &base,
            "back",
            "重印背面",
            "先把要補印背面的紙，照原本的方式放回紙匣再按。",
            sheets
        ),
    );
    c
}

fn reprint_form(base: &str, side: &str, label: &str, note: &str, sheets: u32) -> String {
    let note = if note.is_empty() {
        String::new()
    } else {
        format!("<p class=\"note\">{note}</p>")
    };
    format!(
        "<form class=\"reprint\" method=\"post\" action=\"{base}reprint\">\
         <input type=\"hidden\" name=\"side\" value=\"{side}\">{note}\
         <label>第 <input name=\"first\" type=\"number\" min=\"1\" max=\"{sheets}\" value=\"1\" inputmode=\"numeric\"> 張到第 \
         <input name=\"last\" type=\"number\" min=\"1\" max=\"{sheets}\" value=\"{sheets}\" inputmode=\"numeric\"> 張</label>\
         <button class=\"alt\">{label}</button></form>"
    )
}

fn local_time(epoch: u64) -> String {
    // SAFETY: localtime_r writes only into the struct we pass.
    unsafe {
        // time_t is 64-bit on every target we build for; going through i64
        // avoids libc's deprecated musl time_t alias.
        let t = epoch as i64;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r((&t as *const i64).cast(), &mut tm).is_null() {
            return String::new();
        }
        format!(
            "{}/{} {:02}:{:02}",
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        )
    }
}

const STYLE: &str = r#"
:root{--bg:#f5f5f7;--card:#fff;--fg:#1d1d1f;--mut:#6e6e73;--acc:#0071e3;--bad:#d70015;--warn:#b25000;--ok:#1a7f37;--line:#d2d2d7;--badbg:#fff0f0;--warnbg:#fff7e6}
@media (prefers-color-scheme:dark){:root{--bg:#000;--card:#1c1c1e;--fg:#f5f5f7;--mut:#98989d;--acc:#0a84ff;--bad:#ff453a;--warn:#ff9f0a;--ok:#30d158;--line:#38383a;--badbg:#3a1414;--warnbg:#3a2a10}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 -apple-system,BlinkMacSystemFont,"PingFang TC","Noto Sans CJK TC",sans-serif}
main{max-width:560px;margin:0 auto;padding:24px 16px}
h1{font-size:22px;margin:0 0 4px}
a{color:var(--acc)}
.sub{color:var(--mut);margin:0 0 20px;font-size:14px}
.card{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:16px;margin-bottom:12px}
.t{font-weight:600;word-break:break-all}
.m{color:var(--mut);font-size:14px;margin:4px 0 8px}
dl{display:grid;grid-template-columns:auto 1fr;gap:2px 12px;margin:0 0 12px;font-size:14px}
dt{color:var(--mut)}dd{margin:0}
.hint{font-size:14px;margin:0 0 12px}
.note{font-size:13px;color:var(--mut);margin:8px 0}
.bad{color:var(--bad)}.ok{color:var(--ok)}
.banner{border-radius:12px;padding:12px 16px;margin-bottom:12px;font-size:14px}
.banner.bad{background:var(--badbg);color:var(--bad)}
.banner.warn{background:var(--warnbg);color:var(--warn)}
.banner ul{margin:4px 0 0;padding-left:20px}
button{font:inherit;border:0;border-radius:10px;padding:10px 16px;cursor:pointer}
.go{background:var(--acc);color:#fff;width:100%;font-weight:600;padding:14px}
.go:disabled{opacity:.45;cursor:default}
.alt{background:var(--bg);color:var(--fg);border:1px solid var(--line)}
.x{background:none;color:var(--bad);padding:8px 0;font-size:14px}
details{margin-top:12px;border-top:1px solid var(--line);padding-top:8px}
summary{cursor:pointer;color:var(--mut);font-size:14px}
.reprint{margin:12px 0;display:flex;flex-wrap:wrap;align-items:center;gap:8px;font-size:14px}
.reprint .note{width:100%;margin:0}
.reprint input[type=number]{width:3.5em;font:inherit;padding:4px 6px;border:1px solid var(--line);border-radius:8px;background:var(--bg);color:var(--fg)}
.empty{color:var(--mut);text-align:center;padding:40px 0}
"#;

/// Refresh every 5 s, unless the user is filling in a reprint form.
const SCRIPT: &str = "setInterval(function(){if(!document.querySelector('details[open]')&&\
!(document.activeElement&&document.activeElement.tagName==='INPUT'))location.reload()},5000);";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh-Hant\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body><main>\
         <h1>P1505 手動雙面</h1>\
         <p class=\"sub\">正面印完後，把整疊紙放回紙匣，再按「繼續列印背面」。</p>\
         {body}</main><script>{SCRIPT}</script></body></html>",
        esc(title)
    )
}

fn html(code: u16, body: String) -> Resp {
    Response::from_data(body.into_bytes())
        .with_status_code(code)
        .with_header(header("Content-Type", "text/html; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
}

fn redirect(to: &str) -> Resp {
    Response::from_data(Vec::new())
        .with_status_code(303)
        .with_header(header("Location", to))
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

fn num(form: &HashMap<String, String>, key: &str) -> Result<u32> {
    form.get(key)
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| bad("請輸入紙張編號"))
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out += "&amp;",
            '<' => out += "&lt;",
            '>' => out += "&gt;",
            '"' => out += "&quot;",
            '\'' => out += "&#39;",
            _ => out.push(ch),
        }
    }
    out
}

/// Parse an application/x-www-form-urlencoded body.
fn form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (!k.is_empty()).then(|| (url_decode(k), url_decode(v)))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::JobState::{self, *};
    use crate::status::Side as S;

    fn job(front: JobState, back: JobState) -> DuplexJob {
        DuplexJob {
            id: 5,
            title: "<報告>".into(),
            user: "someone".into(),
            pages: 5,
            sheets: 3,
            created: 0,
            front: Some(S {
                job: 1,
                state: front,
                sheets_done: 0,
            }),
            back: Some(S {
                job: 2,
                state: back,
                sheets_done: 0,
            }),
            stage: Stage::of(Some(front), Some(back), false),
        }
    }

    #[test]
    fn decodes_forms() {
        let f = form("side=back&first=2&last=%33&title=a+b%E5%BC%B5&bad=%zz");
        assert_eq!(f["side"], "back");
        assert_eq!(f["last"], "3");
        assert_eq!(f["title"], "a b張");
        assert_eq!(f["bad"], "%zz");
    }

    #[test]
    fn escapes_html() {
        assert_eq!(
            esc("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }

    #[test]
    fn continue_enabled_only_when_ready() {
        let ready = card(&job(Completed, Held));
        assert!(ready.contains("&lt;報告&gt;"));
        assert!(ready.contains("<button class=\"go\">繼續列印背面"));
        assert!(card(&job(Processing, Held)).contains("<button class=\"go\" disabled>"));
        let mut blocked = job(Completed, Held);
        blocked.stage = Stage::Blocked;
        assert!(card(&blocked).contains("<button class=\"go\" disabled>"));
        assert!(card(&job(Aborted, Held)).contains("正面沒有印完"));
    }

    #[test]
    fn finished_job_offers_removal() {
        let done = card(&job(Completed, Completed));
        assert!(done.contains("雙面列印完成"));
        assert!(done.contains("action=\"jobs/5/remove\""));
    }

    #[test]
    fn error_messages() {
        assert_eq!(message(&bad("x")), "x");
        assert!(
            parse_id("abc")
                .unwrap_err()
                .downcast_ref::<UserError>()
                .unwrap()
                .not_found
        );
        assert!(json_body::<ReprintBody>("{\"side\":\"up\",\"first\":1,\"last\":1}").is_err());
        let b: ReprintBody = json_body("{\"side\":\"back\",\"first\":1,\"last\":2}").unwrap();
        assert_eq!((b.side, b.first, b.last), (Side::Back, 1, 2));
    }
}
