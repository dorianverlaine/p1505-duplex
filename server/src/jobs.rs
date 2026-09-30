//! Sending front and back sides of a record to the real printer.

use crate::ipp::Cups;
use crate::pdf;
use crate::plan::{self, Sheets};
use crate::state::Record;
use anyhow::Result;
use lopdf::Document;

/// Print the fronts of `sheets` now; returns the job id.
pub fn send_fronts(
    cups: &Cups,
    printer: &str,
    rec: &Record,
    src: &Document,
    sheets: Sheets,
) -> Result<i32> {
    let bytes = pdf::select(src, &plan::fronts(sheets), 0)?;
    let title = format!("{} (正面{})", rec.title, range_label(rec, sheets));
    cups.print_pdf(printer, &rec.user, &title, &bytes, false)
}

/// Send the backs of `sheets`, held if `hold`; None when that range has no
/// printable back.
pub fn send_backs(
    cups: &Cups,
    printer: &str,
    rec: &Record,
    src: &Document,
    sheets: Sheets,
    hold: bool,
) -> Result<Option<i32>> {
    let pages = plan::backs(rec.pages, sheets, rec.order);
    if pages.is_empty() {
        return Ok(None);
    }
    let bytes = pdf::select(src, &pages, rec.rotate)?;
    let title = format!("{} (背面{})", rec.title, range_label(rec, sheets));
    cups.print_pdf(printer, &rec.user, &title, &bytes, hold)
        .map(Some)
}

/// " 第 2–3 張" for a partial range, empty for the whole document.
fn range_label(rec: &Record, sheets: Sheets) -> String {
    if sheets == Sheets::all(rec.pages) {
        String::new()
    } else if sheets.first == sheets.last {
        format!(" 第 {} 張", sheets.first)
    } else {
        format!(" 第 {}–{} 張", sheets.first, sheets.last)
    }
}
