//! Manual duplex for the HP LaserJet P1505 on a CUPS server.
//!
//! One binary, three roles:
//! - `duplex` (symlinked into CUPS' backend dir): split jobs into fronts and
//!   held backs;
//! - `p1505-duplex web`: the "continue" page;
//! - `p1505-duplex firmware`: upload the printer firmware with the queue paused.

mod backend;
mod config;
mod firmware;
mod ipp;
mod jobs;
mod pdf;
mod plan;
mod state;
mod web;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: p1505-duplex <command>

commands:
  web              serve the continue page
  firmware         upload the P1505 firmware with the queue paused
  test-pdf [FILE]  write a numbered test document (default duplex-test-5p.pdf)
  backend ARGS     run as the CUPS backend (normally called as `duplex`)

config: /etc/p1505-duplex.conf";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if is_backend(&args) {
        return code(backend::main(&args));
    }

    let result = match args.get(1).map(String::as_str) {
        Some("backend") => return code(backend::main(&args[1..])),
        Some("web") => web::main(),
        Some("firmware") => firmware::main(),
        Some("test-pdf") => test_pdf(args.get(2).map(String::as_str)),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Run as a CUPS backend? cupsd passes the queue name as argv[0] and sets
/// DEVICE_URI; device discovery runs us by path with no arguments.
fn is_backend(args: &[String]) -> bool {
    let uri = std::env::var("DEVICE_URI").unwrap_or_default();
    let name = args
        .first()
        .and_then(|a| Path::new(a).file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    uri.starts_with("duplex:") || name == "duplex"
}

fn code(c: i32) -> ExitCode {
    ExitCode::from(c as u8)
}

fn test_pdf(path: Option<&str>) -> anyhow::Result<()> {
    let path = path.unwrap_or("duplex-test-5p.pdf");
    std::fs::write(path, pdf::test_document(5)?)?;
    println!("{path}");
    Ok(())
}
