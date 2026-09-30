//! CUPS backend for the duplex queue (`duplex:/`).
//!
//! The queue uses a PDF PPD, so CUPS hands us the rendered job as PDF. We keep
//! it for reprints and split it into two jobs on the real printer: the fronts,
//! printed right away, and the backs, held until the user flips the stack and
//! presses "continue" on the web page.
//!
//! Called as `duplex job user title copies options [file]`, or with no
//! arguments for device discovery.

use crate::config::Config;
use crate::ipp::Cups;
use crate::jobs;
use crate::pdf;
use crate::plan::Sheets;
use crate::state::{self, Record, Store};
use anyhow::{Context, Result, bail};
use std::io::Read;

const OK: i32 = 0;
const FAILED: i32 = 1;

pub fn main(args: &[String]) -> i32 {
    if args.len() == 1 {
        println!("direct duplex \"Unknown\" \"Manual duplex\"");
        return OK;
    }
    // Group-writable files so the web page (group lpadmin) can update them.
    // SAFETY: umask only changes this process's file creation mask.
    unsafe { libc::umask(0o002) };
    match run(args) {
        Ok(()) => OK,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            FAILED
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    if !(6..=7).contains(&args.len()) {
        bail!("usage: duplex job user title copies options [file]");
    }
    let id: i32 = args[1].parse().context("job id")?;
    let (user, title) = (args[2].clone(), args[3].clone());

    let mut source = Vec::new();
    match args.get(6) {
        Some(path) => source = std::fs::read(path).with_context(|| format!("reading {path}"))?,
        None => {
            std::io::stdin().read_to_end(&mut source)?;
        }
    }

    let cfg = Config::load()?;
    let doc = pdf::load(&source)?;
    let pages = pdf::page_count(&doc);
    if pages == 0 {
        bail!("document has no pages");
    }
    eprintln!("INFO: {pages} pages");

    let mut rec = Record {
        id,
        title,
        user,
        pages,
        created: state::now(),
        order: cfg.even_order,
        rotate: cfg.even_rotate,
        front_job: None,
        back_job: None,
    };
    let store = Store::new(&cfg.state_dir);
    store.create(&rec, &source)?;

    let cups = Cups::local()?;
    let all = Sheets::all(pages);
    rec.front_job = Some(jobs::send_fronts(&cups, &cfg.printer, &rec, &doc, all)?);
    store.save(&rec)?;
    rec.back_job = jobs::send_backs(&cups, &cfg.printer, &rec, &doc, all, true)?;
    store.save(&rec)?;
    eprintln!(
        "INFO: fronts job {:?}, backs job {:?} (held)",
        rec.front_job, rec.back_job
    );

    if rec.back_job.is_none() {
        // Single page: nothing to continue.
        store.remove(id)?;
    }
    Ok(())
}
