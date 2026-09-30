//! A small IPP client for the local CUPS scheduler.
//!
//! Talks HTTP over CUPS' domain socket. There, cupsd knows the peer's uid, so
//! admin operations authenticate with `Authorization: PeerCred <user>` and
//! succeed when the user is in CUPS' SystemGroup (lpadmin): no password or
//! certificate needed. Print-Job is sent without it so the job is owned by
//! `requesting-user-name`, not by the backend's own user (lp).

use anyhow::{Context, Result, anyhow, bail};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub const SOCKET: &str = "/run/cups/cups.sock";

// Operations.
const PRINT_JOB: u16 = 0x0002;
const CANCEL_JOB: u16 = 0x0008;
const GET_JOB_ATTRIBUTES: u16 = 0x0009;
const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
const RELEASE_JOB: u16 = 0x000D;
const PAUSE_PRINTER: u16 = 0x0010;
const RESUME_PRINTER: u16 = 0x0011;

// Delimiter tags.
const OPERATION: u8 = 0x01;
const JOB: u8 = 0x02;
const END: u8 = 0x03;

// Value tags.
const INTEGER: u8 = 0x21;
const BOOLEAN: u8 = 0x22;
const ENUM: u8 = 0x23;
const TEXT: u8 = 0x41;
const NAME: u8 = 0x42;
const KEYWORD: u8 = 0x44;
const URI: u8 = 0x45;
const CHARSET: u8 = 0x47;
const LANGUAGE: u8 = 0x48;
const MIME: u8 = 0x49;

pub const STATUS_NOT_FOUND: u16 = 0x0406;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Bool(bool),
    Str(String),
    Other(u8, Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub values: Vec<Value>,
}

#[derive(Debug, Default)]
pub struct Response {
    pub status: u16,
    pub attrs: Vec<Attr>,
}

impl Response {
    fn get(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }

    pub fn int(&self, name: &str) -> Option<i32> {
        match self.get(name)?.values.first()? {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        match self.get(name)?.values.first()? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn strs(&self, name: &str) -> Vec<String> {
        self.get(name)
            .map(|a| {
                a.values
                    .iter()
                    .filter_map(|v| match v {
                        Value::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct IppError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for IppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IPP status 0x{:04x}: {}", self.status, self.message)
    }
}

impl std::error::Error for IppError {}

/// An IPP request being built.
pub struct Request {
    buf: Vec<u8>,
    group: u8,
}

impl Request {
    pub fn new(op: u16) -> Request {
        let mut buf = vec![2, 0];
        buf.extend(op.to_be_bytes());
        buf.extend(1u32.to_be_bytes());
        buf.push(OPERATION);
        let mut r = Request {
            buf,
            group: OPERATION,
        };
        r.attr(CHARSET, "attributes-charset", "utf-8");
        r.attr(LANGUAGE, "attributes-natural-language", "en");
        r
    }

    fn raw(&mut self, tag: u8, name: &str, value: &[u8]) -> &mut Self {
        self.buf.push(tag);
        self.buf.extend((name.len() as u16).to_be_bytes());
        self.buf.extend(name.as_bytes());
        self.buf.extend((value.len() as u16).to_be_bytes());
        self.buf.extend(value);
        self
    }

    fn attr(&mut self, tag: u8, name: &str, value: &str) -> &mut Self {
        self.raw(tag, name, value.as_bytes())
    }

    fn int(&mut self, name: &str, value: i32) -> &mut Self {
        self.raw(INTEGER, name, &value.to_be_bytes())
    }

    /// A multi-valued keyword attribute.
    fn keywords(&mut self, name: &str, values: &[&str]) -> &mut Self {
        for (i, v) in values.iter().enumerate() {
            self.attr(KEYWORD, if i == 0 { name } else { "" }, v);
        }
        self
    }

    fn job_group(&mut self) -> &mut Self {
        if self.group != JOB {
            self.buf.push(JOB);
            self.group = JOB;
        }
        self
    }

    fn finish(mut self) -> Vec<u8> {
        self.buf.push(END);
        self.buf
    }
}

pub fn parse(data: &[u8]) -> Result<Response> {
    let mut r = Reader { data, pos: 0 };
    let _version = r.take(2)?;
    let status = u16::from_be_bytes(r.take(2)?.try_into()?);
    let _request_id = r.take(4)?;
    let mut attrs: Vec<Attr> = Vec::new();
    loop {
        let tag = *r.take(1)?.first().unwrap();
        if tag == END {
            break;
        }
        if tag < 0x10 {
            continue; // group delimiter
        }
        let name_len = u16::from_be_bytes(r.take(2)?.try_into()?) as usize;
        let name = String::from_utf8_lossy(r.take(name_len)?).into_owned();
        let value_len = u16::from_be_bytes(r.take(2)?.try_into()?) as usize;
        let raw = r.take(value_len)?;
        let value = match tag {
            INTEGER | ENUM if raw.len() == 4 => Value::Int(i32::from_be_bytes(raw.try_into()?)),
            BOOLEAN if raw.len() == 1 => Value::Bool(raw[0] != 0),
            TEXT | NAME | KEYWORD | URI | CHARSET | LANGUAGE | MIME => {
                Value::Str(String::from_utf8_lossy(raw).into_owned())
            }
            _ => Value::Other(tag, raw.to_vec()),
        };
        match (name.is_empty(), attrs.last_mut()) {
            // An additional value of the previous attribute (1setOf), or a
            // collection member, which we keep flat and never read.
            (true, Some(last)) => last.values.push(value),
            _ => attrs.push(Attr {
                name,
                values: vec![value],
            }),
        }
    }
    Ok(Response { status, attrs })
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or_else(|| anyhow!("truncated IPP response"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
}

/// Connection to cupsd.
pub struct Cups {
    socket: String,
    /// Our own user name, sent as PeerCred for admin operations.
    user: String,
}

impl Cups {
    pub fn local() -> Result<Cups> {
        Ok(Cups {
            socket: SOCKET.into(),
            user: current_user()?,
        })
    }

    fn printer_uri(printer: &str) -> String {
        format!("ipp://localhost/printers/{printer}")
    }

    fn send(&self, path: &str, body: &[u8], admin: bool) -> Result<Response> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("connecting to {}", self.socket))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let mut head = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/ipp\r\n\
             Content-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if admin {
            head += &format!("Authorization: PeerCred {}\r\n", self.user);
        }
        head += "\r\n";
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;

        let (code, payload) = read_http_response(BufReader::new(stream))?;
        match code {
            200 => {}
            401 | 403 => bail!("CUPS refused {path} for user {} (HTTP {code})", self.user),
            _ => bail!("CUPS answered HTTP {code} for {path}"),
        }
        let resp = parse(&payload)?;
        if resp.status >= 0x0400 {
            let message = resp.str("status-message").unwrap_or("").to_string();
            return Err(IppError {
                status: resp.status,
                message,
            }
            .into());
        }
        Ok(resp)
    }

    /// Submit `pdf` to `printer` as `user`; returns the job id.
    pub fn print_pdf(
        &self,
        printer: &str,
        user: &str,
        title: &str,
        pdf: &[u8],
        hold: bool,
    ) -> Result<i32> {
        let mut req = Request::new(PRINT_JOB);
        req.attr(URI, "printer-uri", &Self::printer_uri(printer))
            .attr(NAME, "requesting-user-name", user)
            .attr(NAME, "job-name", title)
            .attr(MIME, "document-format", "application/pdf");
        if hold {
            req.job_group()
                .attr(KEYWORD, "job-hold-until", "indefinite");
        }
        let mut body = req.finish();
        body.extend_from_slice(pdf);
        let resp = self.send(&format!("/printers/{printer}"), &body, false)?;
        resp.int("job-id")
            .ok_or_else(|| anyhow!("Print-Job answer has no job-id"))
    }

    fn job_op(
        &self,
        op: u16,
        printer: &str,
        job: i32,
        extra: impl FnOnce(&mut Request),
    ) -> Result<Response> {
        let mut req = Request::new(op);
        req.attr(URI, "printer-uri", &Self::printer_uri(printer))
            .int("job-id", job)
            .attr(NAME, "requesting-user-name", &self.user);
        extra(&mut req);
        self.send("/jobs/", &req.finish(), true)
    }

    pub fn job(&self, printer: &str, job: i32) -> Result<JobStatus> {
        let resp = self.job_op(GET_JOB_ATTRIBUTES, printer, job, |r| {
            r.keywords(
                "requested-attributes",
                &["job-state", "job-media-sheets-completed"],
            );
        })?;
        Ok(JobStatus {
            state: resp
                .int("job-state")
                .and_then(JobState::from_ipp)
                .unwrap_or(JobState::Pending),
            sheets_done: resp.int("job-media-sheets-completed").unwrap_or(0).max(0) as u32,
        })
    }

    pub fn release(&self, printer: &str, job: i32) -> Result<()> {
        self.job_op(RELEASE_JOB, printer, job, |_| {}).map(drop)
    }

    pub fn cancel(&self, printer: &str, job: i32) -> Result<()> {
        self.job_op(CANCEL_JOB, printer, job, |_| {}).map(drop)
    }

    pub fn printer(&self, printer: &str) -> Result<PrinterStatus> {
        let mut req = Request::new(GET_PRINTER_ATTRIBUTES);
        req.attr(URI, "printer-uri", &Self::printer_uri(printer))
            .attr(NAME, "requesting-user-name", &self.user)
            .keywords(
                "requested-attributes",
                &[
                    "printer-state",
                    "printer-state-reasons",
                    "printer-state-message",
                    "device-uri",
                ],
            );
        let resp = self.send(&format!("/printers/{printer}"), &req.finish(), true)?;
        Ok(PrinterStatus {
            state: resp.int("printer-state").unwrap_or(3),
            reasons: resp
                .strs("printer-state-reasons")
                .into_iter()
                .filter(|r| r != "none")
                .collect(),
            message: resp.str("printer-state-message").unwrap_or("").to_string(),
            device_uri: resp.str("device-uri").unwrap_or("").to_string(),
        })
    }

    /// Stop the queue (like cupsdisable -r message).
    pub fn pause(&self, printer: &str, message: &str) -> Result<()> {
        let mut req = Request::new(PAUSE_PRINTER);
        req.attr(URI, "printer-uri", &Self::printer_uri(printer))
            .attr(NAME, "requesting-user-name", &self.user)
            .attr(TEXT, "printer-state-message", message);
        self.send("/admin/", &req.finish(), true).map(drop)
    }

    pub fn resume(&self, printer: &str) -> Result<()> {
        let mut req = Request::new(RESUME_PRINTER);
        req.attr(URI, "printer-uri", &Self::printer_uri(printer))
            .attr(NAME, "requesting-user-name", &self.user);
        self.send("/admin/", &req.finish(), true).map(drop)
    }
}

/// True when `err` is an IPP "not found" (e.g. a job purged from history).
pub fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<IppError>()
        .is_some_and(|e| e.status == STATUS_NOT_FOUND)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Pending,
    Held,
    Processing,
    Stopped,
    Canceled,
    Aborted,
    Completed,
}

impl JobState {
    fn from_ipp(v: i32) -> Option<JobState> {
        Some(match v {
            3 => JobState::Pending,
            4 => JobState::Held,
            5 => JobState::Processing,
            6 => JobState::Stopped,
            7 => JobState::Canceled,
            8 => JobState::Aborted,
            9 => JobState::Completed,
            _ => return None,
        })
    }

    /// No longer queued: nothing more will come out for this job.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            JobState::Canceled | JobState::Aborted | JobState::Completed
        )
    }
}

#[derive(Debug, Clone)]
pub struct JobStatus {
    pub state: JobState,
    pub sheets_done: u32,
}

#[derive(Debug, Clone)]
pub struct PrinterStatus {
    /// 3 idle, 4 processing, 5 stopped.
    pub state: i32,
    /// printer-state-reasons without "none".
    pub reasons: Vec<String>,
    pub message: String,
    pub device_uri: String,
}

/// Read one HTTP/1.1 response; handles Content-Length and chunked bodies
/// (cupsd may keep the connection open, so reading to EOF would hang).
fn read_http_response(mut r: impl BufRead) -> Result<(u16, Vec<u8>)> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let code: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| anyhow!("bad HTTP status line {line:?}"))?;
    let mut length = None;
    let mut chunked = false;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            bail!("connection closed in HTTP headers");
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "content-length" {
                length = v.parse::<usize>().ok();
            } else if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
                chunked = true;
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            r.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| anyhow!("bad chunk size {line:?}"))?;
            if size == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..])?;
            line.clear();
            r.read_line(&mut line)?; // CRLF after the chunk
        }
    } else if let Some(n) = length {
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    }
    Ok((code, body))
}

fn current_user() -> Result<String> {
    // SAFETY: getpwuid returns a pointer into static storage or null; we copy
    // the name out immediately and this process does not call it concurrently.
    unsafe {
        let pw = libc::getpwuid(libc::geteuid());
        if pw.is_null() {
            bail!("no passwd entry for uid {}", libc::geteuid());
        }
        Ok(std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_string_lossy()
            .into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_request_header_and_attributes() {
        let mut r = Request::new(RELEASE_JOB);
        r.int("job-id", 7);
        let b = r.finish();
        assert_eq!(&b[..9], &[2, 0, 0x00, 0x0D, 0, 0, 0, 1, OPERATION]);
        // attributes-charset first, as IPP requires.
        assert_eq!(b[9], CHARSET);
        assert_eq!(&b[12..30], b"attributes-charset");
        assert_eq!(*b.last().unwrap(), END);
        // A request parses like a response (the op code sits where the status is).
        let parsed = parse(&b).unwrap();
        assert_eq!(parsed.int("job-id"), Some(7));
        assert_eq!(parsed.str("attributes-charset"), Some("utf-8"));
    }

    #[test]
    fn multi_valued_keywords_round_trip() {
        let mut r = Request::new(GET_PRINTER_ATTRIBUTES);
        r.keywords("printer-state-reasons", &["media-jam-error", "paused"]);
        let b = r.finish();
        let resp = parse(&b).unwrap();
        assert_eq!(
            resp.strs("printer-state-reasons"),
            vec!["media-jam-error", "paused"]
        );
    }

    #[test]
    fn hold_goes_in_the_job_group() {
        let mut r = Request::new(PRINT_JOB);
        r.job_group().attr(KEYWORD, "job-hold-until", "indefinite");
        let b = r.finish();
        let pos = b.windows(14).position(|w| w == b"job-hold-until").unwrap();
        // tag byte and 2-byte name length precede the name; JOB delimiter before that.
        assert_eq!(b[pos - 4], JOB);
    }

    #[test]
    fn parses_status_and_values() {
        let mut body = vec![2, 0, 0x04, 0x06, 0, 0, 0, 1, OPERATION];
        let mut r = Request {
            buf: Vec::new(),
            group: OPERATION,
        };
        r.attr(TEXT, "status-message", "Job #9 does not exist.");
        r.raw(ENUM, "job-state", &9i32.to_be_bytes());
        r.raw(BOOLEAN, "printer-is-accepting-jobs", &[1]);
        body.extend(r.buf);
        body.push(END);
        let resp = parse(&body).unwrap();
        assert_eq!(resp.status, STATUS_NOT_FOUND);
        assert_eq!(resp.str("status-message"), Some("Job #9 does not exist."));
        assert_eq!(resp.int("job-state"), Some(9));
        assert_eq!(
            resp.get("printer-is-accepting-jobs").unwrap().values,
            vec![Value::Bool(true)]
        );
    }

    #[test]
    fn truncated_response_is_an_error() {
        assert!(parse(&[2, 0, 0, 0]).is_err());
        assert!(parse(&[2, 0, 0, 0, 0, 0, 0, 1, OPERATION, KEYWORD, 0, 5, b'a']).is_err());
    }

    #[test]
    fn reads_content_length_and_chunked_bodies() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcEXTRA";
        assert_eq!(
            read_http_response(&plain[..]).unwrap(),
            (200, b"abc".to_vec())
        );
        let chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(
            read_http_response(&chunked[..]).unwrap(),
            (200, b"abcde".to_vec())
        );
        let refused = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(read_http_response(&refused[..]).unwrap().0, 401);
    }
}
