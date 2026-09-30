//! `/etc/p1505-duplex.conf`: `KEY=value` lines, `#` comments.

use crate::plan::Order;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub const PATH: &str = "/etc/p1505-duplex.conf";

#[derive(Debug, Clone)]
pub struct Config {
    /// CUPS queue of the real, single-sided printer.
    pub printer: String,
    /// Order of the back sides.
    pub even_order: Order,
    /// Extra rotation of the back sides, 0 or 180.
    pub even_rotate: i64,
    /// Where the web page listens; nginx proxies /duplex/ to it.
    pub listen: String,
    /// Per-job records and source PDFs.
    pub state_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            printer: "HP_LaserJet_P1505".into(),
            even_order: Order::Reverse,
            even_rotate: 180,
            listen: "127.0.0.1:8631".into(),
            state_dir: "/var/spool/p1505-duplex".into(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Config> {
        match std::fs::read_to_string(PATH) {
            Ok(text) => Config::parse(&text).with_context(|| format!("reading {PATH}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {PATH}")),
        }
    }

    pub fn parse(text: &str) -> Result<Config> {
        let mut c = Config::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                bail!("line {}: expected KEY=value", n + 1);
            };
            let value = value.trim();
            match key.trim() {
                "PRINTER" => c.printer = value.into(),
                "EVEN_ORDER" => {
                    c.even_order = value
                        .parse()
                        .map_err(|e| anyhow::anyhow!("line {}: {e}", n + 1))?
                }
                "EVEN_ROTATE" => {
                    c.even_rotate = match value {
                        "0" => 0,
                        "180" => 180,
                        _ => bail!("line {}: EVEN_ROTATE must be 0 or 180", n + 1),
                    }
                }
                "LISTEN" => c.listen = value.into(),
                "STATE_DIR" => c.state_dir = value.into(),
                other => bail!("line {}: unknown key {other}", n + 1),
            }
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_comments() {
        let c = Config::parse(
            "# comment\nPRINTER=Other_Queue\nEVEN_ORDER=forward  # trailing\n\nEVEN_ROTATE=0\n",
        )
        .unwrap();
        assert_eq!(c.printer, "Other_Queue");
        assert_eq!(c.even_order, Order::Forward);
        assert_eq!(c.even_rotate, 0);
        assert_eq!(c.listen, "127.0.0.1:8631");
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("EVEN_ROTATE=90").is_err());
        assert!(Config::parse("EVEN_ORDER=up").is_err());
        assert!(Config::parse("NOPE=1").is_err());
        assert!(Config::parse("just text").is_err());
    }
}
