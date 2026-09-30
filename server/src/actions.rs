//! Things users can do, shared by the web page and the JSON API.

use crate::config::Config;
use crate::ipp::Cups;
use crate::jobs;
use crate::pdf;
use crate::plan::{self, Sheets};
use crate::state::Store;
use anyhow::Result;

/// A problem with the request itself, shown to the user as is.
#[derive(Debug)]
pub struct UserError {
    pub message: String,
    pub not_found: bool,
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UserError {}

fn user(message: impl Into<String>) -> anyhow::Error {
    UserError {
        message: message.into(),
        not_found: false,
    }
    .into()
}

fn not_found(message: impl Into<String>) -> anyhow::Error {
    UserError {
        message: message.into(),
        not_found: true,
    }
    .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Front,
    Back,
}

pub struct Actions<'a> {
    pub cfg: &'a Config,
    pub cups: &'a Cups,
    pub store: &'a Store,
}

impl Actions<'_> {
    fn record(&self, id: i32) -> Result<crate::state::Record> {
        self.store
            .load(id)
            .map_err(|_| not_found("找不到這份工作，可能已經移除"))
    }

    /// Release the held back sides.
    pub fn continue_job(&self, id: i32) -> Result<()> {
        let rec = self.record(id)?;
        let job = rec.back_job.ok_or_else(|| user("這份工作沒有背面"))?;
        self.cups.release(&self.cfg.printer, job)
    }

    /// Reprint one side of sheets `first..=last`. The side's current job is
    /// superseded and canceled if it has not finished (e.g. stopped by a jam).
    pub fn reprint(&self, id: i32, side: Side, first: u32, last: u32) -> Result<()> {
        let mut rec = self.record(id)?;
        let sheets = Sheets::new(first, last, rec.pages)
            .map_err(|_| user(format!("紙張範圍要在第 1 到第 {} 張之間", rec.sheets())))?;
        let src = pdf::load(&self.store.source(id)?)?;
        let printer = &self.cfg.printer;
        match side {
            Side::Front => {
                self.cancel_if_active(rec.front_job)?;
                rec.front_job = Some(jobs::send_fronts(self.cups, printer, &rec, &src, sheets)?);
            }
            Side::Back => {
                if plan::backs(rec.pages, sheets, rec.order).is_empty() {
                    return Err(user("這幾張紙的背面是空白的，不用重印"));
                }
                self.cancel_if_active(rec.back_job)?;
                rec.back_job = jobs::send_backs(self.cups, printer, &rec, &src, sheets, false)?;
            }
        }
        self.store.save(&rec)
    }

    /// Cancel whatever is still printing and forget the job.
    pub fn remove(&self, id: i32) -> Result<()> {
        let rec = self.record(id)?;
        self.cancel_if_active(rec.front_job)?;
        self.cancel_if_active(rec.back_job)?;
        self.store.remove(id)
    }

    fn cancel_if_active(&self, job: Option<i32>) -> Result<()> {
        let Some(job) = job else { return Ok(()) };
        match self.cups.job(&self.cfg.printer, job) {
            Ok(s) if !s.state.is_final() => self.cups.cancel(&self.cfg.printer, job),
            _ => Ok(()),
        }
    }

    /// Cancel any job on the real printer's queue.
    pub fn cancel_queue_job(&self, job: i32) -> Result<()> {
        self.cups.cancel(&self.cfg.printer, job)
    }

    pub fn pause_printer(&self) -> Result<()> {
        self.cups
            .pause(&self.cfg.printer, "paused from the duplex page")
    }

    pub fn resume_printer(&self) -> Result<()> {
        self.cups.resume(&self.cfg.printer)
    }

    /// Print the numbered test document through the duplex queue as `user`.
    pub fn test_page(&self, user_name: &str) -> Result<i32> {
        if user_name.is_empty() || user_name.len() > 64 || user_name.contains(char::is_control) {
            return Err(user("使用者名稱不正確"));
        }
        let pdf = pdf::test_document(5)?;
        self.cups
            .print_pdf(&self.cfg.duplex_queue, user_name, "雙面測試頁", &pdf, false)
    }
}
