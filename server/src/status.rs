//! What the web page, the JSON API and the event stream show: a snapshot of
//! the duplex jobs and the printer, with each job's stage decided in one place.

use crate::config::Config;
use crate::ipp::{self, Cups, JobState, JobStatus, PrinterStatus, QueueJob};
use crate::state::{self, Record, Store};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Human-readable job state for the page.
///
/// No page progress: CUPS' sheet counters depend on the driver's filter
/// chain (with foomatic drivers two filters each report every page), so
/// they cannot be shown as "N of M".
pub trait JobStateText {
    fn text(self) -> &'static str;
}

impl JobStateText for JobState {
    fn text(self) -> &'static str {
        match self {
            JobState::Pending => "排隊中",
            JobState::Held => "等待繼續",
            JobState::Processing => "列印中",
            JobState::Stopped => "已停止",
            JobState::Canceled => "已取消",
            JobState::Aborted => "失敗",
            JobState::Completed => "已完成",
        }
    }
}

/// Where a duplex job stands, from the user's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Fronts not finished yet; backs held.
    FrontPrinting,
    /// Fronts stopped, aborted or canceled; backs still held.
    FrontFailed,
    /// Fronts done, but the printer cannot print right now.
    Blocked,
    /// Fronts done: flip the stack and continue.
    ReadyToFlip,
    BackPrinting,
    BackFailed,
    Done,
    /// A job's state could not be read.
    Unknown,
}

impl Stage {
    pub fn of(front: Option<JobState>, back: Option<JobState>, blocked: bool) -> Stage {
        use JobState::*;
        match back {
            Some(Held) => match front {
                Some(Completed) if blocked => Stage::Blocked,
                Some(Completed) => Stage::ReadyToFlip,
                Some(Aborted | Canceled | Stopped) => Stage::FrontFailed,
                Some(Pending | Held | Processing) => Stage::FrontPrinting,
                None => Stage::Unknown,
            },
            Some(Pending | Processing) => Stage::BackPrinting,
            Some(Completed) => Stage::Done,
            Some(Aborted | Canceled | Stopped) => Stage::BackFailed,
            None => Stage::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Side {
    pub job: i32,
    pub state: JobState,
    pub sheets_done: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplexJob {
    pub id: i32,
    pub title: String,
    pub user: String,
    pub pages: u32,
    pub sheets: u32,
    pub created: u64,
    pub front: Option<Side>,
    pub back: Option<Side>,
    pub stage: Stage,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Report,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reason {
    pub keyword: String,
    pub text: String,
    pub severity: Severity,
}

/// The printer as the job list needs it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Printer {
    pub name: String,
    /// idle, processing or stopped.
    pub state: &'static str,
    pub reasons: Vec<Reason>,
    pub message: String,
    /// Printing is not possible right now.
    pub blocking: bool,
    pub accepting: bool,
    pub queued: u32,
    pub usb_connected: bool,
    pub state_changed: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub printer: Option<Printer>,
    pub cups_error: Option<String>,
    /// Newest first.
    pub jobs: Vec<DuplexJob>,
}

/// Result of the last firmware upload, written by `p1505-duplex firmware`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FirmwareRecord {
    pub time: u64,
    pub ok: bool,
    pub attempts: u32,
    pub error: Option<String>,
}

pub const FIRMWARE_FILE: &str = "firmware.json";

/// Everything about the printer, for the status view.
#[derive(Debug, Clone, Serialize)]
pub struct PrinterDetails {
    pub printer: Option<Printer>,
    pub cups_error: Option<String>,
    pub make_model: String,
    pub info: String,
    pub location: String,
    pub device_uri: String,
    pub markers: Vec<Marker>,
    pub firmware: Option<FirmwareRecord>,
    pub duplex_queue: Option<Printer>,
    /// Jobs waiting or printing on the real printer.
    pub queue: Vec<QueueJob>,
    /// Recently finished jobs, newest first.
    pub history: Vec<QueueJob>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Marker {
    pub name: String,
    /// Percent, or null when the printer does not report it.
    pub level: Option<i32>,
}

pub struct Reader<'a> {
    pub cfg: &'a Config,
    pub cups: &'a Cups,
    pub store: &'a Store,
}

impl Reader<'_> {
    /// Current state of all duplex jobs. Also drops records older than a day
    /// whose jobs are all finished.
    pub fn snapshot(&self) -> Snapshot {
        let (status, cups_error) = match self.cups.printer(&self.cfg.printer) {
            Ok(p) => (Some(p), None),
            Err(e) => {
                eprintln!("printer status: {e:#}");
                (None, Some(format!("{e:#}")))
            }
        };
        let printer = status
            .as_ref()
            .map(|p| printer(&self.cfg.printer, p, usb_connected(&self.cfg.usb_id)));
        let blocked = printer.as_ref().is_some_and(|p| p.blocking);

        let mut jobs = Vec::new();
        for rec in self.store.list() {
            let front = self.side(rec.front_job);
            let back = self.side(rec.back_job);
            let active = [&front, &back]
                .iter()
                .any(|s| s.as_ref().is_some_and(|s| !s.state.is_final()));
            if !active && state::now().saturating_sub(rec.created) > state::KEEP_SECS {
                if let Err(e) = self.store.remove(rec.id) {
                    eprintln!("removing old record {}: {e:#}", rec.id);
                }
                continue;
            }
            jobs.push(duplex_job(&rec, front, back, blocked));
        }
        jobs.reverse();
        Snapshot {
            printer,
            cups_error,
            jobs,
        }
    }

    fn side(&self, job: Option<i32>) -> Option<Side> {
        let job = job?;
        let status = match self.cups.job(&self.cfg.printer, job) {
            Ok(s) => s,
            // Purged from CUPS' history: it finished long ago.
            Err(e) if ipp::is_not_found(&e) => JobStatus {
                state: JobState::Completed,
                sheets_done: 0,
            },
            Err(e) => {
                eprintln!("job {job}: {e:#}");
                return None;
            }
        };
        Some(Side {
            job,
            state: status.state,
            sheets_done: status.sheets_done,
        })
    }

    pub fn details(&self) -> PrinterDetails {
        let usb = usb_connected(&self.cfg.usb_id);
        let (status, cups_error) = match self.cups.printer(&self.cfg.printer) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let p = status.clone().unwrap_or_default();
        let list = |finished, limit| {
            self.cups
                .jobs(&self.cfg.printer, finished, limit)
                .unwrap_or_else(|e| {
                    eprintln!("listing jobs: {e:#}");
                    Vec::new()
                })
        };
        let mut history = list(true, 30);
        history.sort_by_key(|j| std::cmp::Reverse((j.completed.unwrap_or(j.created), j.id)));
        PrinterDetails {
            printer: status.as_ref().map(|s| printer(&self.cfg.printer, s, usb)),
            cups_error,
            make_model: p.make_model,
            info: p.info,
            location: p.location,
            device_uri: p.device_uri,
            markers: p
                .markers
                .into_iter()
                .map(|(name, level)| Marker {
                    name,
                    level: (level >= 0).then_some(level),
                })
                .collect(),
            firmware: read_firmware(&self.cfg.state_dir),
            duplex_queue: self
                .cups
                .printer(&self.cfg.duplex_queue)
                .ok()
                .map(|s| printer(&self.cfg.duplex_queue, &s, usb)),
            queue: list(false, 50),
            history,
        }
    }
}

fn duplex_job(rec: &Record, front: Option<Side>, back: Option<Side>, blocked: bool) -> DuplexJob {
    let stage = Stage::of(
        front.as_ref().map(|s| s.state),
        back.as_ref().map(|s| s.state),
        blocked,
    );
    DuplexJob {
        id: rec.id,
        title: rec.title.clone(),
        user: rec.user.clone(),
        pages: rec.pages,
        sheets: rec.sheets(),
        created: rec.created,
        front,
        back,
        stage,
    }
}

pub fn printer(name: &str, p: &PrinterStatus, usb: bool) -> Printer {
    Printer {
        name: name.to_string(),
        state: match p.state {
            4 => "processing",
            5 => "stopped",
            _ => "idle",
        },
        reasons: p
            .reasons
            .iter()
            .map(|r| Reason {
                keyword: r.clone(),
                text: reason_text(r),
                severity: severity(r),
            })
            .collect(),
        message: p.message.clone(),
        blocking: blocking(p),
        accepting: p.accepting,
        queued: p.queued,
        usb_connected: usb,
        state_changed: p.state_changed,
    }
}

/// Printer-state-reasons that stop printing (keyword without its suffix).
pub fn blocking(p: &PrinterStatus) -> bool {
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

fn severity(reason: &str) -> Severity {
    if reason.ends_with("-warning") {
        Severity::Warning
    } else if reason.ends_with("-report") {
        Severity::Report
    } else if reason.ends_with("-error")
        || matches!(
            base(reason),
            "media-jam"
                | "media-empty"
                | "media-needed"
                | "door-open"
                | "cover-open"
                | "offline"
                | "connecting-to-device"
        )
    {
        Severity::Error
    } else {
        Severity::Warning
    }
}

pub fn reason_text(reason: &str) -> String {
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
        "cups-waiting-for-job-completed" => "等待工作完成".into(),
        other => other.into(),
    }
}

/// Is a USB device `vendor:product` (hex, as in lsusb) attached?
pub fn usb_connected(id: &str) -> bool {
    usb_connected_in(Path::new("/sys/bus/usb/devices"), id)
}

fn usb_connected_in(root: &Path, id: &str) -> bool {
    let Some((vendor, product)) = id.split_once(':') else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    let read = |p: &Path, f: &str| {
        std::fs::read_to_string(p.join(f))
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default()
    };
    entries.flatten().any(|e| {
        let p = e.path();
        read(&p, "idVendor") == vendor.to_ascii_lowercase()
            && read(&p, "idProduct") == product.to_ascii_lowercase()
    })
}

pub fn read_firmware(state_dir: &Path) -> Option<FirmwareRecord> {
    let data = std::fs::read(state_dir.join(FIRMWARE_FILE)).ok()?;
    serde_json::from_slice(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use JobState::*;

    #[test]
    fn stages() {
        assert_eq!(
            Stage::of(Some(Processing), Some(Held), false),
            Stage::FrontPrinting
        );
        assert_eq!(
            Stage::of(Some(Pending), Some(Held), false),
            Stage::FrontPrinting
        );
        assert_eq!(
            Stage::of(Some(Completed), Some(Held), false),
            Stage::ReadyToFlip
        );
        assert_eq!(Stage::of(Some(Completed), Some(Held), true), Stage::Blocked);
        assert_eq!(
            Stage::of(Some(Aborted), Some(Held), false),
            Stage::FrontFailed
        );
        assert_eq!(
            Stage::of(Some(Stopped), Some(Held), true),
            Stage::FrontFailed
        );
        assert_eq!(
            Stage::of(Some(Completed), Some(Processing), false),
            Stage::BackPrinting
        );
        assert_eq!(
            Stage::of(Some(Completed), Some(Completed), false),
            Stage::Done
        );
        assert_eq!(
            Stage::of(Some(Completed), Some(Canceled), false),
            Stage::BackFailed
        );
        assert_eq!(Stage::of(None, Some(Held), false), Stage::Unknown);
        assert_eq!(Stage::of(Some(Completed), None, false), Stage::Unknown);
    }

    fn status(state: i32, reasons: &[&str]) -> PrinterStatus {
        PrinterStatus {
            state,
            reasons: reasons.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn blocking_reasons() {
        assert!(!blocking(&status(3, &[])));
        assert!(!blocking(&status(3, &["toner-low-warning"])));
        assert!(blocking(&status(3, &["media-jam-error"])));
        assert!(blocking(&status(4, &["media-empty"])));
        assert!(blocking(&status(3, &["connecting-to-device"])));
        assert!(blocking(&status(5, &["paused"])));
    }

    #[test]
    fn reasons_get_text_and_severity() {
        let p = printer(
            "P",
            &status(3, &["media-jam-error", "toner-low-warning", "x-report"]),
            true,
        );
        let got: Vec<(&str, Severity)> = p
            .reasons
            .iter()
            .map(|r| (r.text.as_str(), r.severity.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("卡紙了，請打開印表機取出卡住的紙", Severity::Error),
                ("碳粉不足", Severity::Warning),
                ("x", Severity::Report),
            ]
        );
        assert_eq!(reason_text("door-open-warning"), "機蓋沒有關好");
        assert_eq!(p.state, "idle");
        assert!(p.usb_connected);
    }

    #[test]
    fn finds_usb_devices() {
        let root = std::env::temp_dir().join(format!("p1505-usb-test-{}", std::process::id()));
        let dev = root.join("2-1");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("idVendor"), "03f0\n").unwrap();
        std::fs::write(dev.join("idProduct"), "3f17\n").unwrap();
        std::fs::create_dir_all(root.join("usb1")).unwrap();
        assert!(usb_connected_in(&root, "03f0:3f17"));
        assert!(usb_connected_in(&root, "03F0:3F17"));
        assert!(!usb_connected_in(&root, "03f0:0001"));
        assert!(!usb_connected_in(&root, "garbage"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn snapshot_json_shape() {
        let rec = Record {
            id: 5,
            title: "t".into(),
            user: "u".into(),
            pages: 5,
            created: 1,
            order: crate::plan::Order::Reverse,
            rotate: 180,
            front_job: Some(1),
            back_job: Some(2),
        };
        let side = |job, state| {
            Some(Side {
                job,
                state,
                sheets_done: 3,
            })
        };
        let job = duplex_job(&rec, side(1, Completed), side(2, Held), false);
        let json = serde_json::to_value(&job).unwrap();
        assert_eq!(json["stage"], "ready_to_flip");
        assert_eq!(json["sheets"], 3);
        assert_eq!(json["front"]["state"], "completed");
        assert_eq!(json["back"]["job"], 2);
    }
}
