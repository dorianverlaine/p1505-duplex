//! The "continue" page, served behind nginx at /duplex/.
//!
//! Lists duplex jobs with the live state of their front and back jobs and of
//! the printer, releases held back sides, and reprints ranges of sheets after
//! a jam or a double feed. Runs as p1505-duplex (group lpadmin) so it may act
//! on anyone's job.

use crate::config::Config;
use crate::ipp::{self, Cups, JobState, JobStatus, PrinterStatus};
use crate::jobs;
use crate::pdf;
use crate::plan::{self, Sheets};
use crate::state::{self, Record, Store};
use anyhow::{Result, anyhow, bail};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read;
use tiny_http::{Header, Method, Response, Server};

pub fn main() -> Result<()> {
    let cfg = Config::load()?;
    let server =
        Server::http(&cfg.listen).map_err(|e| anyhow!("listening on {}: {e}", cfg.listen))?;
    eprintln!("listening on {}", cfg.listen);
    let app = App {
        cups: Cups::local()?,
        store: Store::new(&cfg.state_dir),
        cfg,
    };
    for mut req in server.incoming_requests() {
        let mut body = String::new();
        let _ = req.as_reader().take(64 * 1024).read_to_string(&mut body);
        let resp = app.handle(req.method(), req.url(), &body);
        let _ = req.respond(resp);
    }
    Ok(())
}

type Resp = Response<std::io::Cursor<Vec<u8>>>;

struct App {
    cfg: Config,
    cups: Cups,
    store: Store,
}

impl App {
    fn handle(&self, method: &Method, url: &str, body: &str) -> Resp {
        let path = url.split('?').next().unwrap_or("/");
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let result = match (method, parts.as_slice()) {
            (Method::Get, [""]) | (Method::Get, ["index.html"]) => Ok(html(200, self.index())),
            (Method::Post, ["jobs", id, action]) => match id.parse::<i32>() {
                Ok(id) => self
                    .act(id, action, &form(body))
                    .map(|()| redirect("../../")),
                Err(_) => Err(anyhow!("找不到這份工作")),
            },
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
            eprintln!("{method} {url}: {e:#}");
            let body = format!(
                "<div class=\"card\"><p class=\"bad\">{}</p><p><a href=\"{}\">回到列表</a></p></div>",
                esc(&format!("{e:#}")),
                if parts.len() == 3 { "../../" } else { "./" },
            );
            html(500, page("發生錯誤", &body))
        })
    }

    fn act(&self, id: i32, action: &str, form: &HashMap<String, String>) -> Result<()> {
        let mut rec = self
            .store
            .load(id)
            .map_err(|_| anyhow!("找不到這份工作，可能已經移除"))?;
        let printer = &self.cfg.printer;
        match action {
            "continue" => {
                let job = rec.back_job.ok_or_else(|| anyhow!("這份工作沒有背面"))?;
                self.cups.release(printer, job)?;
            }
            "reprint" => {
                let first = num(form, "first")?;
                let last = num(form, "last")?;
                let sheets = Sheets::new(first, last, rec.pages)
                    .map_err(|_| anyhow!("紙張範圍要在第 1 到第 {} 張之間", rec.sheets()))?;
                let src = pdf::load(&self.store.source(id)?)?;
                match form.get("side").map(String::as_str) {
                    // A reprint supersedes the side's current job, which must
                    // not keep printing (e.g. a job stopped by a jam).
                    Some("front") => {
                        self.cancel_if_active(rec.front_job)?;
                        rec.front_job =
                            Some(jobs::send_fronts(&self.cups, printer, &rec, &src, sheets)?);
                    }
                    Some("back") => {
                        if plan::backs(rec.pages, sheets, rec.order).is_empty() {
                            bail!("這幾張紙的背面是空白的，不用重印");
                        }
                        self.cancel_if_active(rec.back_job)?;
                        rec.back_job =
                            jobs::send_backs(&self.cups, printer, &rec, &src, sheets, false)?;
                    }
                    _ => bail!("不明的列印面"),
                }
                self.store.save(&rec)?;
            }
            "remove" => {
                self.cancel_if_active(rec.front_job)?;
                self.cancel_if_active(rec.back_job)?;
                self.store.remove(id)?;
            }
            _ => bail!("不明的動作"),
        }
        Ok(())
    }

    /// Cancel `job` unless it already finished (or is gone).
    fn cancel_if_active(&self, job: Option<i32>) -> Result<()> {
        let Some(job) = job else { return Ok(()) };
        match self.cups.job(&self.cfg.printer, job) {
            Ok(s) if !s.state.is_final() => self.cups.cancel(&self.cfg.printer, job),
            _ => Ok(()),
        }
    }

    fn job_status(&self, job: Option<i32>) -> Option<JobStatus> {
        let job = job?;
        match self.cups.job(&self.cfg.printer, job) {
            Ok(s) => Some(s),
            // Purged from CUPS' history: it finished long ago.
            Err(e) if ipp::is_not_found(&e) => Some(JobStatus {
                state: JobState::Completed,
                sheets_done: 0,
            }),
            Err(e) => {
                eprintln!("job {job}: {e:#}");
                None
            }
        }
    }

    fn index(&self) -> String {
        let mut out = String::new();
        let printer = match self.cups.printer(&self.cfg.printer) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("printer status: {e:#}");
                out += "<div class=\"banner bad\">連不上 CUPS，無法取得印表機狀態。</div>";
                None
            }
        };
        let blocked = printer.as_ref().is_some_and(blocking);
        if let Some(p) = &printer {
            out += &printer_banner(p);
        }

        let mut cards = Vec::new();
        for rec in self.store.list() {
            let front = self.job_status(rec.front_job);
            let back = self.job_status(rec.back_job);
            let active = [&front, &back]
                .iter()
                .any(|s| s.as_ref().is_some_and(|s| !s.state.is_final()));
            if !active && state::now().saturating_sub(rec.created) > state::KEEP_SECS {
                if let Err(e) = self.store.remove(rec.id) {
                    eprintln!("removing old record {}: {e:#}", rec.id);
                }
                continue;
            }
            cards.push(card(&rec, front.as_ref(), back.as_ref(), blocked));
        }
        if cards.is_empty() {
            out += "<div class=\"empty\">沒有進行中的雙面列印工作</div>";
        }
        for c in cards.iter().rev() {
            out += c;
        }
        page("P1505 手動雙面", &out)
    }
}

/// Printer-state-reasons that stop printing (keyword without its suffix).
fn blocking(p: &PrinterStatus) -> bool {
    p.state == 5
        || p.reasons.iter().any(|r| {
            r.ends_with("-error")
                || matches!(
                    base(r),
                    "media-jam"
                        | "media-empty"
                        | "media-needed"
                        | "door-open"
                        | "cover-open"
                        | "offline"
                        | "connecting-to-device"
                )
        })
}

fn base(reason: &str) -> &str {
    reason
        .strip_suffix("-error")
        .or_else(|| reason.strip_suffix("-warning"))
        .or_else(|| reason.strip_suffix("-report"))
        .unwrap_or(reason)
}

fn reason_text(reason: &str) -> String {
    match base(reason) {
        "media-jam" => "卡紙了，請打開印表機取出卡住的紙".into(),
        "media-empty" | "media-needed" => "紙匣沒有紙".into(),
        "media-low" => "紙快用完了".into(),
        "door-open" | "cover-open" => "機蓋沒有關好".into(),
        "toner-low" | "marker-supply-low" => "碳粉不足".into(),
        "toner-empty" | "marker-supply-empty" => "碳粉用完了".into(),
        "offline" => "印表機離線".into(),
        "connecting-to-device" => "找不到印表機，請確認電源和 USB 線".into(),
        "paused" => "列印佇列已暫停".into(),
        other => other.into(),
    }
}

fn printer_banner(p: &PrinterStatus) -> String {
    if p.reasons.is_empty() && p.state != 5 {
        return String::new();
    }
    let mut items: Vec<String> = p.reasons.iter().map(|r| reason_text(r)).collect();
    if !p.message.is_empty() {
        items.push(p.message.clone());
    }
    if items.is_empty() {
        items.push("印表機已停止".into());
    }
    let class = if blocking(p) { "bad" } else { "warn" };
    let list: String = items
        .iter()
        .map(|i| format!("<li>{}</li>", esc(i)))
        .collect();
    format!("<div class=\"banner {class}\"><strong>印表機狀態</strong><ul>{list}</ul></div>")
}

fn state_text(s: Option<&JobStatus>, sheets: u32) -> String {
    let Some(s) = s else {
        return "狀態不明".into();
    };
    match s.state {
        JobState::Pending => "排隊中".into(),
        JobState::Held => "等待繼續".into(),
        JobState::Processing if s.sheets_done > 0 => {
            format!("列印中 · 已印 {}/{} 張", s.sheets_done, sheets)
        }
        JobState::Processing => "列印中".into(),
        JobState::Stopped => "已停止".into(),
        JobState::Canceled => "已取消".into(),
        JobState::Aborted => "失敗".into(),
        JobState::Completed => "已完成".into(),
    }
}

fn card(
    rec: &Record,
    front: Option<&JobStatus>,
    back: Option<&JobStatus>,
    blocked: bool,
) -> String {
    let sheets = rec.sheets();
    let when = local_time(rec.created);
    let fstate = front.map(|s| s.state);
    let bstate = back.map(|s| s.state);
    let front_done = fstate == Some(JobState::Completed);
    let front_failed = matches!(
        fstate,
        Some(JobState::Aborted | JobState::Canceled | JobState::Stopped)
    );

    let mut c = String::new();
    let _ = write!(
        c,
        "<div class=\"card\"><div class=\"t\">{}</div>\
         <div class=\"m\">{} 頁 · {} 張紙 · {} · {}</div>\
         <dl><dt>正面</dt><dd>{}</dd><dt>背面</dt><dd>{}</dd></dl>",
        esc(&rec.title),
        rec.pages,
        sheets,
        esc(&rec.user),
        when,
        state_text(front, sheets),
        state_text(back, sheets),
    );

    let base = format!("jobs/{}/", rec.id);
    match bstate {
        Some(JobState::Held) => {
            let (hint, enabled) = if front_failed {
                ("<p class=\"hint bad\">正面沒有印完。請用下方「出問題了？」補印正面，再繼續。</p>".to_string(), false)
            } else if !front_done {
                (
                    "<p class=\"hint\">正面還在列印，印完後再繼續。</p>".to_string(),
                    false,
                )
            } else if blocked {
                (
                    "<p class=\"hint bad\">印表機目前無法列印，排除問題後再繼續。</p>".to_string(),
                    false,
                )
            } else {
                (
                    format!(
                        "<p class=\"hint\">正面已印完：取出這 {sheets} 張紙，放回紙匣後按「繼續」。</p>"
                    ),
                    true,
                )
            };
            c += &hint;
            let _ = write!(
                c,
                "<form method=\"post\" action=\"{base}continue\"><button class=\"go\"{}>繼續列印背面</button></form>",
                if enabled { "" } else { " disabled" }
            );
        }
        Some(JobState::Completed) => {
            c += "<p class=\"hint ok\">雙面列印完成。</p>";
            let _ = write!(
                c,
                "<form method=\"post\" action=\"{base}remove\"><button class=\"go\">完成，移除這份工作</button></form>"
            );
        }
        Some(JobState::Aborted | JobState::Canceled | JobState::Stopped) => {
            c += "<p class=\"hint bad\">背面沒有印完。如果有紙卡住或一次進了兩張，用下方「出問題了？」補印。</p>";
        }
        _ => {}
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
        .ok_or_else(|| anyhow!("請輸入紙張編號"))
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

    fn printer(state: i32, reasons: &[&str]) -> PrinterStatus {
        PrinterStatus {
            state,
            reasons: reasons.iter().map(|r| r.to_string()).collect(),
            message: String::new(),
            device_uri: String::new(),
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
    fn blocking_reasons() {
        assert!(!blocking(&printer(3, &[])));
        assert!(!blocking(&printer(3, &["toner-low-warning"])));
        assert!(blocking(&printer(3, &["media-jam-error"])));
        assert!(blocking(&printer(4, &["media-empty"])));
        assert!(blocking(&printer(3, &["connecting-to-device"])));
        assert!(blocking(&printer(5, &["paused"])));
    }

    #[test]
    fn reason_texts() {
        assert_eq!(
            reason_text("media-jam-error"),
            "卡紙了，請打開印表機取出卡住的紙"
        );
        assert_eq!(reason_text("door-open-warning"), "機蓋沒有關好");
        assert_eq!(reason_text("something-new-report"), "something-new");
    }

    fn rec() -> Record {
        Record {
            id: 5,
            title: "<報告>".into(),
            user: "someone".into(),
            pages: 5,
            created: 0,
            order: crate::plan::Order::Reverse,
            rotate: 180,
            front_job: Some(1),
            back_job: Some(2),
        }
    }

    fn st(state: JobState) -> JobStatus {
        JobStatus {
            state,
            sheets_done: 0,
        }
    }

    #[test]
    fn continue_enabled_only_when_front_done_and_printer_ok() {
        let held = st(JobState::Held);
        let ready = card(&rec(), Some(&st(JobState::Completed)), Some(&held), false);
        assert!(ready.contains("&lt;報告&gt;"));
        assert!(ready.contains("<button class=\"go\">繼續列印背面"));
        let printing = card(&rec(), Some(&st(JobState::Processing)), Some(&held), false);
        assert!(printing.contains("<button class=\"go\" disabled>"));
        let jammed = card(&rec(), Some(&st(JobState::Completed)), Some(&held), true);
        assert!(jammed.contains("<button class=\"go\" disabled>"));
        let failed = card(&rec(), Some(&st(JobState::Aborted)), Some(&held), false);
        assert!(failed.contains("正面沒有印完"));
    }

    #[test]
    fn finished_job_offers_removal() {
        let done = card(
            &rec(),
            Some(&st(JobState::Completed)),
            Some(&st(JobState::Completed)),
            false,
        );
        assert!(done.contains("雙面列印完成"));
        assert!(done.contains("action=\"jobs/5/remove\""));
    }
}
