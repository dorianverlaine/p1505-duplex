//! One directory per duplex job under the state dir, shared between the
//! backend (runs as lp) and the web page (runs as p1505-duplex, group
//! lpadmin). The state dir is setgid lpadmin and files are group-writable.
//!
//! ```text
//! <state_dir>/<id>/record.json   Record below
//! <state_dir>/<id>/source.pdf    the whole job, kept for reprints
//! ```

use crate::plan::{self, Order};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Records older than this are removed once none of their jobs is active.
pub const KEEP_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Job id on the duplex queue; also the directory name.
    pub id: i32,
    pub title: String,
    pub user: String,
    pub pages: u32,
    pub created: u64,
    /// Back-side layout used for this job, fixed at submission so reprints
    /// match even if the config changes meanwhile.
    pub order: Order,
    pub rotate: i64,
    /// Current front-side job on the real printer (latest reprint wins).
    pub front_job: Option<i32>,
    /// Current back-side job; held until the user continues.
    pub back_job: Option<i32>,
}

impl Record {
    pub fn sheets(&self) -> u32 {
        plan::sheet_count(self.pages)
    }
}

pub struct Store {
    dir: PathBuf,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    fn job_dir(&self, id: i32) -> PathBuf {
        self.dir.join(id.to_string())
    }

    pub fn source_path(&self, id: i32) -> PathBuf {
        self.job_dir(id).join("source.pdf")
    }

    /// Create the directory for a new job and store its source PDF.
    pub fn create(&self, record: &Record, source: &[u8]) -> Result<()> {
        let dir = self.job_dir(record.id);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        write_atomic(&dir.join("source.pdf"), source)?;
        self.save(record)
    }

    pub fn save(&self, record: &Record) -> Result<()> {
        let json = serde_json::to_vec_pretty(record)?;
        write_atomic(&self.job_dir(record.id).join("record.json"), &json)
    }

    pub fn load(&self, id: i32) -> Result<Record> {
        let path = self.job_dir(id).join("record.json");
        let data = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(serde_json::from_slice(&data)?)
    }

    pub fn source(&self, id: i32) -> Result<Vec<u8>> {
        let path = self.source_path(id);
        fs::read(&path).with_context(|| format!("reading {}", path.display()))
    }

    /// All records, oldest first. Unreadable entries are skipped.
    pub fn list(&self) -> Vec<Record> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<Record> = entries
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
            .filter_map(|id| self.load(id).ok())
            .collect();
        out.sort_by_key(|r| (r.created, r.id));
        out
    }

    pub fn remove(&self, id: i32) -> Result<()> {
        let dir = self.job_dir(id);
        fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: i32, created: u64) -> Record {
        Record {
            id,
            title: "報告.pdf".into(),
            user: "someone".into(),
            pages: 5,
            created,
            order: Order::Reverse,
            rotate: 180,
            front_job: Some(10),
            back_job: Some(11),
        }
    }

    #[test]
    fn create_list_update_remove() {
        let dir = std::env::temp_dir().join(format!("p1505-state-test-{}", std::process::id()));
        let store = Store::new(&dir);
        store.create(&record(2, 200), b"%PDF-2").unwrap();
        store.create(&record(1, 100), b"%PDF-1").unwrap();
        fs::create_dir_all(dir.join("not-a-job")).unwrap();

        let ids: Vec<i32> = store.list().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(store.source(2).unwrap(), b"%PDF-2");

        let mut r = store.load(1).unwrap();
        assert_eq!(r.title, "報告.pdf");
        assert_eq!(r.sheets(), 3);
        r.back_job = Some(42);
        store.save(&r).unwrap();
        assert_eq!(store.load(1).unwrap().back_job, Some(42));

        store.remove(1).unwrap();
        assert_eq!(store.list().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
