//! Firmware upload for the HP LaserJet P1505.
//!
//! The P1505 loses its firmware on every power-on and silently drops jobs
//! until the host uploads it again. HPLIP tries on plug-in, but when a job is
//! already waiting CUPS grabs the device first and the upload fails with
//! "Device busy". So: pause the queue, upload with hp-firmware, resume the
//! queue (always, even if the upload failed). Run by udev via systemd, as root.

use crate::config::Config;
use crate::ipp::Cups;
use anyhow::{Context, Result, bail};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

const ATTEMPTS: u32 = 6;

pub fn main() -> Result<()> {
    let cfg = Config::load()?;
    let cups = Cups::local()?;
    let uri = cups.printer(&cfg.printer)?.device_uri;
    if uri.is_empty() {
        bail!("{} has no device-uri", cfg.printer);
    }

    cups.pause(&cfg.printer, "loading firmware")?;
    let result = upload(&uri);
    let resumed = cups.resume(&cfg.printer);
    match (&result, resumed) {
        (_, Err(e)) => eprintln!("could not resume {}: {e:#}", cfg.printer),
        (Ok(n), Ok(())) => eprintln!("firmware loaded (attempt {n})"),
        (Err(_), Ok(())) => {}
    }
    result.map(drop)
}

fn upload(uri: &str) -> Result<u32> {
    // Give the device a moment after enumeration.
    sleep(Duration::from_secs(3));
    for attempt in 1..=ATTEMPTS {
        let status = Command::new("hp-firmware")
            .args(["-n", uri])
            .status()
            .context("running hp-firmware")?;
        if status.success() {
            sleep(Duration::from_secs(2));
            return Ok(attempt);
        }
        eprintln!("hp-firmware failed (attempt {attempt}/{ATTEMPTS}): {status}");
        sleep(Duration::from_secs(5));
    }
    bail!("firmware upload failed after {ATTEMPTS} attempts")
}
