//! Daily plan-approval jobs and the one-shot queue that already lived in
//! `scheduled.json`.
//!
//! A one-shot job is claimed and deleted when it comes due. A plan job stays.
//! When it comes due, this module writes one [`PlanRun`] and moves `next_run_at`
//! forward. Approving a run only marks items; the host turns those items into
//! background tasks and is responsible for not starting two of them at once.
//!
//! The summary is built from local task records. It does not call a model, so a
//! plan is either a real item list or an explicit empty list, never free text
//! the queue cannot render.

use std::cell::{Cell, RefCell};
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{Task, Turn, Workspace};

const DEFAULT_TEMPLATE: &str =
    "Complete the task \"{task.title}\".\nContext: {task.context}\nDone when: {task.done_when}";
const PREAMBLE: &str =
    "This task was approved from the daily plan. Tool calls are pre-approved.\n\n";
const MAX_ITEMS: usize = 20;

thread_local! {
    static DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct DirGuard {
    prev: Option<PathBuf>,
}

impl DirGuard {
    fn push(path: PathBuf) -> Self {
        let prev = DIR_OVERRIDE.with(|slot| slot.replace(Some(path)));
        DirGuard { prev }
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        let prev = self.prev.take();
        DIR_OVERRIDE.with(|slot| *slot.borrow_mut() = prev);
    }
}

/// Exclusive lock for `scheduled.json` and `plans.json`.
///
/// The window poll and `evobuddy schedule --run-due` both read, modify, and
/// write those files. The lock is held across one whole update. A second
/// acquire on the same thread nests instead of locking the file again.
struct LockGuard {
    file: Option<std::fs::File>,
}

impl LockGuard {
    fn acquire() -> io::Result<Self> {
        Self::acquire_with(false)
    }

    /// `nonblock = true` fails with `WouldBlock` instead of waiting when the
    /// files are locked elsewhere. The window poll and the headless command
    /// both prefer a short, bounded wait over an indefinite one.
    fn acquire_with(nonblock: bool) -> io::Result<Self> {
        let depth = LOCK_DEPTH.with(Cell::get);
        if depth > 0 {
            LOCK_DEPTH.with(|slot| slot.set(depth + 1));
            return Ok(Self { file: None });
        }
        let path = dir().join("schedule.lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        lock_exclusive(&file, nonblock)?;
        LOCK_DEPTH.with(|slot| slot.set(1));
        Ok(Self { file: Some(file) })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let depth = LOCK_DEPTH.with(Cell::get);
        if depth > 0 {
            LOCK_DEPTH.with(|slot| slot.set(depth - 1));
        }
        if depth == 1 {
            if let Some(file) = self.file.take() {
                let _ = unlock_exclusive(&file);
            }
        }
    }
}

/// Take the state lock without waiting. `None` means another process holds
/// it; callers skip the pass (the next GUI poll or OS timer run retries).
fn try_lock() -> Option<LockGuard> {
    match LockGuard::acquire_with(true) {
        Ok(guard) => Some(guard),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            eprintln!("scheduler: schedule files are busy; skipping this pass");
            None
        }
        Err(error) => {
            eprintln!("scheduler: could not lock schedule files: {error}");
            None
        }
    }
}

/// Run `f` while holding the state lock, without waiting for it. `None`
/// means another process holds the files. The headless command retries this
/// a few times and reports a distinct exit code when they stay busy.
pub fn with_state_lock<T>(f: impl FnOnce() -> T) -> Option<T> {
    let _guard = try_lock()?;
    Some(f())
}

#[cfg(unix)]
fn lock_exclusive(file: &std::fs::File, nonblock: bool) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let flags = if nonblock { libc::LOCK_EX | libc::LOCK_NB } else { libc::LOCK_EX };
    let result = unsafe { libc::flock(file.as_raw_fd(), flags) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_exclusive(file: &std::fs::File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn lock_exclusive(file: &std::fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let mut overlapped = WindowsOverlapped::default();
    let flags = if nonblock { 2 | 1 } else { 2 };
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as *mut core::ffi::c_void,
            flags,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn unlock_exclusive(file: &std::fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let mut overlapped = WindowsOverlapped::default();
    let ok = unsafe {
        UnlockFileEx(
            file.as_raw_handle() as *mut core::ffi::c_void,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
#[repr(C)]
struct WindowsOverlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut core::ffi::c_void,
}

#[cfg(windows)]
impl Default for WindowsOverlapped {
    fn default() -> Self {
        Self {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
extern "system" {
    fn LockFileEx(
        file: *mut core::ffi::c_void,
        flags: u32,
        reserved: u32,
        bytes_low: u32,
        bytes_high: u32,
        overlapped: *mut WindowsOverlapped,
    ) -> i32;
    fn UnlockFileEx(
        file: *mut core::ffi::c_void,
        reserved: u32,
        bytes_low: u32,
        bytes_high: u32,
        overlapped: *mut WindowsOverlapped,
    ) -> i32;
}

fn dir() -> PathBuf {
    if let Some(path) = DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return path;
    }
    if let Ok(state) = std::env::var("EB_STATE") {
        let path = PathBuf::from(state);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                return parent.to_path_buf();
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".evobuddy")
}

fn jobs_path() -> PathBuf {
    dir().join("scheduled.json")
}

fn plans_path() -> PathBuf {
    dir().join("plans.json")
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
}

/// Seconds east of UTC for `unix`, or 0 where local time is unavailable.
pub fn local_offset_secs(unix: u64) -> i32 {
    #[cfg(unix)]
    unsafe {
        let t = unix as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i32
    }
    #[cfg(not(unix))]
    {
        let _ = unix;
        0
    }
}

pub fn format_local(unix: u64) -> String {
    #[cfg(unix)]
    unsafe {
        let t = unix as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return unix.to_string();
        }
        return format!(
            "{:02}-{:02} {:02}:{:02}",
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        );
    }
    #[cfg(not(unix))]
    {
        unix.to_string()
    }
}

pub fn parse_hhmm(text: &str) -> Option<u32> {
    let (hour, minute) = text.trim().split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}

const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

fn describe_periodic(periodic: Option<&Periodic>) -> String {
    let Some(p) = periodic else {
        return "periodic".into();
    };
    let time = format_hhmm(p.minutes);
    match p.freq.as_str() {
        "day" => format!("daily {time}"),
        "week" | "biweek" => {
            let days: Vec<&str> = if p.byday.is_empty() {
                WEEKDAYS.to_vec()
            } else {
                p.byday.iter().map(|&d| WEEKDAYS[d as usize]).collect()
            };
            let prefix = if p.freq == "biweek" { "biweekly" } else { "weekly" };
            format!("{prefix} {} · {time}", days.join(","))
        }
        "month" => format!("monthly day {} · {time}", join_nums(&p.bymonthday)),
        "year" => format!(
            "yearly {}/{} · {time}",
            p.bymonth,
            join_nums(&p.bymonthday)
        ),
        _ => "periodic".into(),
    }
}

fn join_nums(nums: &[u32]) -> String {
    nums.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

pub fn format_hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The next `minutes`-past-midnight strictly after `now`, in Unix seconds.
pub fn next_daily_run(now: u64, minutes: u32, offset: i32) -> u64 {
    let local = now as i64 + offset as i64;
    let day = local.div_euclid(86_400);
    let sec = local.rem_euclid(86_400);
    let target = minutes as i64 * 60;
    let day = if sec < target { day } else { day + 1 };
    let local_target = day * 86_400 + target;
    (local_target - offset as i64).max(0) as u64
}

fn local_midnight(unix: u64, offset: i32) -> u64 {
    let local = unix as i64 + offset as i64;
    let day = local.div_euclid(86_400);
    (day * 86_400 - offset as i64).max(0) as u64
}

/// The latest daily boundary at `minutes` that is still `<= now`.
pub fn most_recent_slot(now: u64, minutes: u32, offset: i32) -> u64 {
    let local = now as i64 + offset as i32 as i64;
    let day = local.div_euclid(86_400);
    let target = minutes as i64 * 60;
    let today = day * 86_400 + target;
    let slot_local = if local >= today { today } else { today - 86_400 };
    (slot_local - offset as i64).max(0) as u64
}

#[derive(Clone, Debug)]
struct Job {
    id: String,
    kind: String,
    title: String,
    prompt: String,
    due_at: u64,
    daily_minutes: u32,
    /// For `interval` prompt jobs: hours between runs.
    interval_hours: u32,
    /// For `periodic` prompt jobs: the recurrence.
    periodic: Option<Periodic>,
    next_run_at: u64,
    /// Set by Run now. Consumed by the next tick so a missing timestamp cannot
    /// look like a request to run immediately.
    force: bool,
    deadline_minutes: u32,
    template: String,
    suggest: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    pub context: String,
    pub done_when: String,
    pub source: String,
    pub prompt: String,
    pub agent_route: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    pub selected: bool,
    pub approved: bool,
    pub skipped: bool,
    pub started: bool,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanRun {
    pub id: String,
    pub job_id: String,
    pub title: String,
    pub created_at: u64,
    pub deadline_at: u64,
    /// `pending`, `approved`, `partial`, or `skipped`.
    pub status: String,
    /// Empty, `timeout_skip_merge`, `user_skip`, `approved`, or `partial`.
    pub resolution: String,
    pub summary: String,
    pub diff: String,
    pub items: Vec<PlanItem>,
}

#[derive(Clone, Debug)]
pub struct TaskSnap {
    pub title: String,
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    pub agent_route: Option<String>,
    pub updated_at: Option<u64>,
    pub streaming: bool,
    pub failed: bool,
    pub queued: bool,
    pub plan_item_id: Option<String>,
    pub excerpt: String,
}

#[derive(Clone, Debug)]
pub struct JobRow {
    pub index: usize,
    pub title: String,
    pub prompt: String,
    pub when: String,
    pub plan: bool,
}

#[derive(Clone, Debug)]
pub enum Tick {
    RunPrompt { title: String, prompt: String },
    PlanReady { title: String, items: usize },
    PlanExpired { title: String },
}

#[derive(Clone, Copy, Debug)]
pub enum DecisionKind {
    Selected,
    All,
    Skip,
}

#[derive(Clone, Debug)]
pub struct DispatchItem {
    pub item_id: String,
    pub title: String,
    pub prompt: String,
    pub workspace_id: Option<String>,
    pub agent_route: Option<String>,
}

pub fn snapshots(tasks: &[Task], workspaces: &[Workspace]) -> Vec<TaskSnap> {
    tasks
        .iter()
        .filter(|task| !task.archived)
        .map(|task| {
            let workspace_name = task
                .workspace_id
                .as_deref()
                .and_then(|id| workspaces.iter().find(|workspace| workspace.id == id))
                .map(|workspace| workspace.name.clone())
                .unwrap_or_default();
            let (streaming, failed, excerpt) = turn_flags(task);
            TaskSnap {
                title: task.title.clone(),
                workspace_id: task.workspace_id.clone(),
                workspace_name,
                agent_route: task.agent_route.clone(),
                updated_at: task.updated_at,
                streaming,
                failed,
                queued: task.dispatch_queued,
                plan_item_id: task.plan_item_id.clone(),
                excerpt,
            }
        })
        .collect()
}

fn turn_flags(task: &Task) -> (bool, bool, String) {
    let mut streaming = false;
    let mut failed = task.failure_unseen;
    let mut excerpt = String::new();
    for turn in &task.turns {
        match turn {
            Turn::User { text, .. } => {
                excerpt = text.chars().take(240).collect();
            }
            Turn::Agent {
                status, streaming: live, ..
            } => {
                streaming = *live;
                if status.as_deref().is_some_and(|text| text.starts_with("Failed")) {
                    failed = true;
                }
            }
        }
    }
    (streaming, failed, excerpt)
}

pub fn tick(now: u64, snaps: &[TaskSnap]) -> Vec<Tick> {
    tick_inner(now, local_offset_secs(now), snaps)
}

/// Expire and generate due plans. One-shot jobs stay in the file, and no
/// session is started. `evobuddy schedule --run-due` uses this so a closed
/// window still produces a plan the next launch can dispatch.
pub fn tick_plans(now: u64, snaps: &[TaskSnap]) -> Vec<Tick> {
    let Some(_lock) = try_lock() else {
        return Vec::new();
    };
    plans_due(now, local_offset_secs(now), snaps)
}

/// Remove due one-shot jobs and return their prompts. Does not generate a plan.
pub fn claim_once_prompts() -> Vec<(String, String)> {
    let Some(_lock) = try_lock() else {
        return Vec::new();
    };
    claim_once(unix_now())
        .into_iter()
        .filter_map(|event| match event {
            Tick::RunPrompt { title, prompt } => Some((title, prompt)),
            _ => None,
        })
        .collect()
}

fn tick_at(dir: &Path, now: u64, offset: i32, snaps: &[TaskSnap]) -> Vec<Tick> {
    let _guard = DirGuard::push(dir.to_path_buf());
    tick_inner(now, offset, snaps)
}

fn tick_plans_at(dir: &Path, now: u64, offset: i32, snaps: &[TaskSnap]) -> Vec<Tick> {
    let _guard = DirGuard::push(dir.to_path_buf());
    let Some(_lock) = try_lock() else {
        return Vec::new();
    };
    plans_due(now, offset, snaps)
}

fn tick_inner(now: u64, offset: i32, snaps: &[TaskSnap]) -> Vec<Tick> {
    let Some(_lock) = try_lock() else {
        return Vec::new();
    };
    let mut events = Vec::new();
    events.extend(claim_once(now));
    events.extend(plans_due(now, offset, snaps));
    events
}

fn plans_due(now: u64, offset: i32, snaps: &[TaskSnap]) -> Vec<Tick> {
    let mut events = Vec::new();
    events.extend(expire_pending(now));
    events.extend(generate_due(now, offset, snaps));
    events
}

pub fn add_once(title: &str, prompt: &str, delay_secs: u64) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let mut jobs = load_jobs();
    let due = unix_now().saturating_add(delay_secs);
    jobs.push(Job {
        id: String::new(),
        kind: "once".into(),
        title: title.to_string(),
        prompt: prompt.to_string(),
        due_at: due,
        daily_minutes: 0,
        interval_hours: 0,
        periodic: None,
        next_run_at: 0,
        force: false,
        deadline_minutes: 45,
        template: String::new(),
        suggest: Vec::new(),
    });
    save_jobs(&jobs);
}

pub fn add_plan(title: &str, daily_minutes: u32, suggest: &[String]) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let now = unix_now();
    let offset = local_offset_secs(now);
    let mut jobs = load_jobs();
    jobs.push(Job {
        id: fresh_id("job"),
        kind: "plan".into(),
        title: title.to_string(),
        prompt: String::new(),
        due_at: 0,
        daily_minutes,
        interval_hours: 0,
        periodic: None,
        next_run_at: next_daily_run(now, daily_minutes, offset),
        force: false,
        deadline_minutes: 45,
        template: DEFAULT_TEMPLATE.to_string(),
        suggest: suggest.to_vec(),
    });
    save_jobs(&jobs);
}

/// A recurring prompt schedule beyond daily/interval: which days, weeks,
/// months it lands on. `byday` is 0=Mon..6=Sun; empty means every day.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Periodic {
    pub freq: String, // `day` | `week` | `biweek` | `month` | `year`
    #[serde(default)]
    pub minutes: u32,
    #[serde(default)]
    pub byday: Vec<u8>,
    #[serde(default)]
    pub bymonthday: Vec<u32>,
    #[serde(default)]
    pub bymonth: u32,
    /// Local week index at creation; `biweek` keeps its parity.
    #[serde(default)]
    pub anchor_week: u64,
}

/// Weekday of `local_days` as 0=Mon..6=Sun (1970-01-01 was a Thursday).
fn weekday_monday0(local_days: i64) -> u8 {
    (local_days + 3).rem_euclid(7) as u8
}

/// (year, month 1-12, day 1-31) for days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    // Jan/Feb belong to the previous solar year in this calendar.
    (y + i64::from(m <= 2), m, d)
}

fn periodic_day_matches(periodic: &Periodic, local_days: i64) -> bool {
    match periodic.freq.as_str() {
        "day" => true,
        "week" => {
            periodic.byday.is_empty() || periodic.byday.contains(&weekday_monday0(local_days))
        }
        "biweek" => {
            let in_week = periodic.byday.is_empty()
                || periodic.byday.contains(&weekday_monday0(local_days));
            let parity = (local_days.div_euclid(7) as u64)
                .wrapping_sub(periodic.anchor_week)
                % 2
                == 0;
            in_week && parity
        }
        "month" => {
            let (_, _, dom) = civil_from_days(local_days);
            !periodic.bymonthday.is_empty() && periodic.bymonthday.contains(&dom)
        }
        "year" => {
            let (_, month, dom) = civil_from_days(local_days);
            month == periodic.bymonth
                && !periodic.bymonthday.is_empty()
                && periodic.bymonthday.contains(&dom)
        }
        _ => false,
    }
}

/// The next run of a periodic schedule strictly after `now`, in Unix seconds.
pub fn next_periodic_run(now: u64, periodic: &Periodic, offset: i32) -> Option<u64> {
    let local_now = now as i64 + offset as i64;
    let today = local_now.div_euclid(86_400);
    for day in today..today + 1_500 {
        if !periodic_day_matches(periodic, day) {
            continue;
        }
        let candidate = day * 86_400 + periodic.minutes as i64 * 60;
        if candidate > local_now {
            return Some((candidate - offset as i64).max(0) as u64);
        }
    }
    None
}

/// The editable per-item prompt template shown in the plan dialog.
pub fn default_template() -> &'static str {
    DEFAULT_TEMPLATE
}

/// A prompt that runs every day at `minutes` past local midnight.
pub fn add_daily(title: &str, prompt: &str, minutes: u32) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let now = unix_now();
    let offset = local_offset_secs(now);
    let mut jobs = load_jobs();
    jobs.push(Job {
        id: fresh_id("job"),
        kind: "daily".into(),
        title: title.to_string(),
        prompt: prompt.to_string(),
        due_at: 0,
        daily_minutes: minutes,
        interval_hours: 0,
        periodic: None,
        next_run_at: next_daily_run(now, minutes, offset),
        force: false,
        deadline_minutes: 45,
        template: String::new(),
        suggest: Vec::new(),
    });
    save_jobs(&jobs);
}

/// A prompt that runs every `hours` hours from now.
pub fn add_interval(title: &str, prompt: &str, hours: u32) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let now = unix_now();
    let mut jobs = load_jobs();
    jobs.push(Job {
        id: fresh_id("job"),
        kind: "interval".into(),
        title: title.to_string(),
        prompt: prompt.to_string(),
        due_at: 0,
        daily_minutes: 0,
        interval_hours: hours.clamp(1, 24),
        periodic: None,
        next_run_at: now + u64::from(hours.clamp(1, 24)) * 3600,
        force: false,
        deadline_minutes: 45,
        template: String::new(),
        suggest: Vec::new(),
    });
    save_jobs(&jobs);
}

/// A one-shot prompt at the next occurrence of `minutes` past local midnight.
pub fn add_once_at(title: &str, prompt: &str, minutes: u32) {
    let now = unix_now();
    let offset = local_offset_secs(now);
    let due = next_daily_run(now, minutes, offset);
    add_once(title, prompt, due.saturating_sub(now));
}

/// Full plan creation with the configurable template and approval deadline.
pub fn add_plan_full(
    title: &str,
    daily_minutes: u32,
    suggest: &[String],
    template: &str,
    deadline_minutes: u32,
) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let now = unix_now();
    let offset = local_offset_secs(now);
    let mut jobs = load_jobs();
    jobs.push(Job {
        id: fresh_id("job"),
        kind: "plan".into(),
        title: title.to_string(),
        prompt: String::new(),
        due_at: 0,
        daily_minutes,
        interval_hours: 0,
        periodic: None,
        next_run_at: next_daily_run(now, daily_minutes, offset),
        force: false,
        deadline_minutes: deadline_minutes.clamp(1, 24 * 60),
        template: if template.trim().is_empty() {
            DEFAULT_TEMPLATE.to_string()
        } else {
            template.to_string()
        },
        suggest: suggest.to_vec(),
    });
    save_jobs(&jobs);
}

/// A recurring prompt on a weekly/biweekly/monthly/yearly grid.
pub fn add_periodic(title: &str, prompt: &str, periodic: &Periodic) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let now = unix_now();
    let offset = local_offset_secs(now);
    let Some(next) = next_periodic_run(now, periodic, offset) else {
        return;
    };
    let mut jobs = load_jobs();
    jobs.push(Job {
        id: fresh_id("job"),
        kind: "periodic".into(),
        title: title.to_string(),
        prompt: prompt.to_string(),
        due_at: 0,
        daily_minutes: 0,
        interval_hours: 0,
        periodic: Some(periodic.clone()),
        next_run_at: next,
        force: false,
        deadline_minutes: 45,
        template: String::new(),
        suggest: Vec::new(),
    });
    save_jobs(&jobs);
}

pub fn remove_at(index: usize) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let mut jobs = load_jobs();
    if index < jobs.len() {
        jobs.remove(index);
        save_jobs(&jobs);
    }
}

/// Make job `index` due on the next tick. A plan that already has a pending
/// run is left alone; the caller should show that the queue is occupied.
pub fn run_now(index: usize) -> bool {
    let Some(_lock) = try_lock() else {
        return false;
    };
    let mut jobs = load_jobs();
    let Some(job) = jobs.get_mut(index) else {
        return false;
    };
    if job.kind == "plan" {
        if pending_for(&job.id).is_some() {
            return false;
        }
        job.force = true;
    } else if job.kind == "daily" || job.kind == "interval" || job.kind == "periodic" {
        job.next_run_at = 0;
    } else {
        job.due_at = 0;
    }
    save_jobs(&jobs);
    true
}

pub fn rows() -> Vec<JobRow> {
    let now = unix_now();
    load_jobs()
        .into_iter()
        .enumerate()
        .map(|(index, job)| {
            let when = match job.kind.as_str() {
                "plan" => {
                    let due = if job.force || job.next_run_at <= now {
                        "due now".to_string()
                    } else {
                        format!("in {}s", job.next_run_at - now)
                    };
                    format!("daily {} · {due}", format_hhmm(job.daily_minutes))
                }
                "daily" => {
                    let due = if job.next_run_at <= now {
                        "due now".to_string()
                    } else {
                        format!("in {}s", job.next_run_at - now)
                    };
                    format!("daily {} · {due}", format_hhmm(job.daily_minutes))
                }
                "interval" => {
                    let due = if job.next_run_at <= now {
                        "due now".to_string()
                    } else {
                        format!("in {}s", job.next_run_at - now)
                    };
                    format!("every {}h · {due}", job.interval_hours)
                }
                "periodic" => {
                    let due = if job.next_run_at <= now {
                        "due now".to_string()
                    } else {
                        format!("in {}s", job.next_run_at - now)
                    };
                    format!(
                        "{} · {due}",
                        describe_periodic(job.periodic.as_ref())
                    )
                }
                _ if job.due_at <= now => "due now".to_string(),
                _ => format!("in {}s", job.due_at - now),
            };
            JobRow {
                index,
                title: job.title,
                prompt: match job.kind.as_str() {
                    "plan" => "Plan, approve, then dispatch.".into(),
                    "daily" | "interval" => job.prompt.clone(),
                    _ => job.prompt,
                },
                when,
                plan: job.kind == "plan",
            }

        })
        .collect()
}

pub fn plans() -> Vec<PlanRun> {
    load_plans()
}

pub fn pending_count() -> usize {
    load_plans()
        .iter()
        .filter(|plan| plan.status == "pending")
        .count()
}

pub fn set_selected(plan_id: &str, item_id: &str, selected: bool) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let mut plans = load_plans();
    if let Some(plan) = plans.iter_mut().find(|plan| plan.id == plan_id && plan.status == "pending")
    {
        if let Some(item) = plan.items.iter_mut().find(|item| item.id == item_id) {
            item.selected = selected;
        }
    }
    save_plans(&plans);
}

pub fn set_prompt(plan_id: &str, item_id: &str, prompt: &str) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let mut plans = load_plans();
    if let Some(plan) = plans.iter_mut().find(|plan| plan.id == plan_id && plan.status == "pending")
    {
        if let Some(item) = plan.items.iter_mut().find(|item| item.id == item_id && !item.started)
        {
            item.prompt = prompt.to_string();
        }
    }
    save_plans(&plans);
}

/// Approve or skip a pending plan. Already-decided plans are left unchanged.
pub fn decide(plan_id: &str, kind: DecisionKind) -> usize {
    let Some(_lock) = try_lock() else {
        return 0;
    };
    let mut plans = load_plans();
    let Some(plan) = plans.iter_mut().find(|plan| plan.id == plan_id) else {
        return 0;
    };
    if plan.status != "pending" {
        return 0;
    }
    match kind {
        DecisionKind::Skip => {
            for item in &mut plan.items {
                item.approved = false;
                item.skipped = true;
            }
            plan.status = "skipped".into();
            plan.resolution = "user_skip".into();
            save_plans(&plans);
            0
        }
        DecisionKind::Selected | DecisionKind::All => {
            let choose_all = matches!(kind, DecisionKind::All);
            if !choose_all && plan.items.iter().all(|item| !item.selected) {
                return 0;
            }
            let mut queued = 0;
            for item in &mut plan.items {
                let take = choose_all || item.selected;
                if take {
                    item.approved = true;
                    item.skipped = false;
                    item.selected = true;
                    if !item.started {
                        queued += 1;
                    }
                } else {
                    item.approved = false;
                    item.skipped = true;
                }
            }
            let all = plan.items.iter().all(|item| item.approved);
            plan.status = if all { "approved" } else { "partial" }.into();
            plan.resolution = plan.status.clone();
            save_plans(&plans);
            queued
        }
    }
}

pub fn dispatch_queue() -> Vec<DispatchItem> {
    let mut items = Vec::new();
    for plan in load_plans() {
        for item in plan.items {
            if item.approved && !item.skipped && !item.started {
                items.push(DispatchItem {
                    item_id: item.id,
                    title: item.title,
                    prompt: item.prompt,
                    workspace_id: item.workspace_id,
                    agent_route: item.agent_route,
                });
            }
        }
    }
    items
}

pub fn note_session(item_id: &str, session_id: &str) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let mut plans = load_plans();
    for plan in &mut plans {
        if let Some(item) = plan.items.iter_mut().find(|item| item.id == item_id) {
            item.session_id = Some(session_id.to_string());
        }
    }
    save_plans(&plans);
}

pub fn mark_started(item_id: &str) {
    let Some(_lock) = try_lock() else {
        return;
    };
    let mut plans = load_plans();
    for plan in &mut plans {
        if let Some(item) = plan.items.iter_mut().find(|item| item.id == item_id) {
            item.started = true;
        }
    }
    save_plans(&plans);
}

fn claim_once(now: u64) -> Vec<Tick> {
    let offset = local_offset_secs(now);
    let mut jobs = load_jobs();
    let mut due = Vec::new();
    for job in jobs.iter_mut() {
        match job.kind.as_str() {
            "plan" => {}
            "once" if job.due_at <= now => {
                due.push(Tick::RunPrompt {
                    title: job.title.clone(),
                    prompt: job.prompt.clone(),
                });
                job.due_at = u64::MAX; // claimed; removed below
            }
            "daily" if job.next_run_at <= now => {
                due.push(Tick::RunPrompt {
                    title: job.title.clone(),
                    prompt: job.prompt.clone(),
                });
                job.next_run_at = next_daily_run(now, job.daily_minutes, offset);
            }
            "interval" if job.next_run_at <= now => {
                due.push(Tick::RunPrompt {
                    title: job.title.clone(),
                    prompt: job.prompt.clone(),
                });
                job.next_run_at =
                    now + u64::from(job.interval_hours.clamp(1, 24)) * 3600;
            }
            "periodic" if job.next_run_at <= now => {
                due.push(Tick::RunPrompt {
                    title: job.title.clone(),
                    prompt: job.prompt.clone(),
                });
                if let Some(periodic) = job.periodic.as_ref() {
                    job.next_run_at = next_periodic_run(now, periodic, offset)
                        .unwrap_or(u64::MAX);
                }
            }
            _ => {}
        }
    }
    let keep: Vec<Job> = jobs
        .into_iter()
        .filter(|job| {
            job.kind == "plan"
                || job.kind == "daily"
                || job.kind == "interval"
                || job.kind == "periodic"
                || job.due_at != u64::MAX
        })
        .collect();
    save_jobs(&keep);
    due
}

fn expire_pending(now: u64) -> Vec<Tick> {
    let mut plans = load_plans();
    let mut events = Vec::new();
    let mut changed = false;
    for plan in &mut plans {
        if plan.status == "pending" && now >= plan.deadline_at {
            for item in &mut plan.items {
                if !item.approved {
                    item.skipped = true;
                }
            }
            plan.status = "skipped".into();
            plan.resolution = "timeout_skip_merge".into();
            events.push(Tick::PlanExpired {
                title: plan.title.clone(),
            });
            changed = true;
        }
    }
    if changed {
        save_plans(&plans);
    }
    events
}

fn generate_due(now: u64, offset: i32, snaps: &[TaskSnap]) -> Vec<Tick> {
    let mut jobs = load_jobs();
    let mut events = Vec::new();
    let mut changed = false;
    for job in &mut jobs {
        if job.kind != "plan" {
            continue;
        }
        let due = job.force || job.next_run_at <= now;
        if !due {
            continue;
        }
        if pending_for(&job.id).is_some() {
            // One waiting plan occupies the slot. Move the clock forward so
            // the next tick does not try to write a second plan today.
            job.force = false;
            job.next_run_at = next_daily_run(now, job.daily_minutes, offset);
            changed = true;
            continue;
        }
        let manual = job.force;
        let plan = build_plan(job, now, offset, snaps, manual);
        events.push(Tick::PlanReady {
            title: plan.title.clone(),
            items: plan.items.len(),
        });
        let mut plans = load_plans();
        plans.push(plan);
        save_plans(&plans);
        job.force = false;
        job.next_run_at = next_daily_run(now, job.daily_minutes, offset);
        changed = true;
    }
    if changed {
        save_jobs(&jobs);
    }
    events
}

fn build_plan(job: &Job, now: u64, offset: i32, snaps: &[TaskSnap], manual: bool) -> PlanRun {
    let (start, end) = if manual {
        let end = now;
        let start = local_midnight(now, offset).saturating_sub(86_400);
        (start, end)
    } else {
        let slot = most_recent_slot(now, job.daily_minutes, offset);
        let today_start = local_midnight(slot, offset).saturating_sub(86_400);
        let missed_start = if job.next_run_at > 0 && job.next_run_at < slot {
            local_midnight(job.next_run_at, offset).saturating_sub(86_400)
        } else {
            today_start
        };
        (missed_start.min(today_start), slot.max(job.next_run_at.min(now)))
    };
    let end = if manual { now } else { end.max(1) };
    let previous = latest_plan(&job.id);
    let mut items = Vec::new();
    let mut undated = 0usize;
    for snap in snaps {
        if snap.updated_at.is_none() {
            undated += 1;
        }
        let Some(at) = snap.updated_at else { continue };
        if at < start || at >= end {
            continue;
        }
        if !(snap.streaming || snap.failed || snap.queued) {
            continue;
        }
        if items.iter().any(|item: &PlanItem| item.title == snap.title) {
            continue;
        }
        items.push(item_from_snap(job, snap, &previous));
        if items.len() == MAX_ITEMS {
            break;
        }
    }
    if items.len() < MAX_ITEMS {
        if let Some(previous) = &previous {
            for old in &previous.items {
                if items.len() == MAX_ITEMS {
                    break;
                }
                if !should_carry(old, snaps) {
                    continue;
                }
                if items.iter().any(|item| item.title == old.title) {
                    continue;
                }
                items.push(carried_item(job, old));
            }
        }
    }
    for line in &job.suggest {
        let title = line.trim();
        if title.is_empty() || items.len() == MAX_ITEMS {
            continue;
        }
        if items.iter().any(|item| item.title == title) {
            continue;
        }
        items.push(template_item(job, title));
    }
    let in_window: Vec<&TaskSnap> = snaps
        .iter()
        .filter(|snap| snap.updated_at.is_some_and(|at| at >= start && at < end))
        .collect();
    let completed = in_window
        .iter()
        .filter(|snap| !snap.streaming && !snap.failed && !snap.queued)
        .count();
    let unfinished = in_window.len() - completed;
    let timed_out = previous
        .as_ref()
        .is_some_and(|plan| plan.resolution == "timeout_skip_merge");
    let mut summary = vec![format!(
        "Sessions in the window: {}. Completed {completed}. Still open {unfinished}.",
        in_window.len()
    )];
    if undated > 0 {
        summary.push(format!("{undated} sessions had no timestamp and were left out."));
    }
    if timed_out {
        summary.push(
            "The previous plan timed out. Its remaining items were merged here.".into(),
        );
    }
    if items.is_empty() {
        summary.push("Nothing to dispatch.".into());
    }
    let diff = diff_line(previous.as_ref(), &items);
    let id = fresh_id("plan");
    for (index, item) in items.iter_mut().enumerate() {
        item.id = format!("{id}-{index}");
    }
    PlanRun {
        id,
        job_id: job.id.clone(),
        title: job.title.clone(),
        created_at: now,
        deadline_at: now.saturating_add(job.deadline_minutes as u64 * 60),
        status: "pending".into(),
        resolution: String::new(),
        summary: summary.join("\n"),
        diff,
        items,
    }
}

fn item_from_snap(job: &Job, snap: &TaskSnap, previous: &Option<PlanRun>) -> PlanItem {
    let carried = snap.plan_item_id.is_some()
        || previous.as_ref().is_some_and(|plan| {
            plan.items.iter().any(|item| item.title == snap.title)
        });
    let source = if carried { "carried" } else { "unfinished" };
    let context = if snap.excerpt.is_empty() {
        "No recent message.".to_string()
    } else {
        snap.excerpt.clone()
    };
    draft_item(
        job,
        &snap.title,
        &context,
        "The session finishes without an error.",
        source,
        snap.agent_route.clone(),
        snap.workspace_id.clone(),
        &snap.workspace_name,
    )
}

fn should_carry(old: &PlanItem, snaps: &[TaskSnap]) -> bool {
    if old.skipped {
        return !finished_link(&old.id, snaps);
    }
    if old.approved && !finished_link(&old.id, snaps) {
        return true;
    }
    false
}

fn finished_link(item_id: &str, snaps: &[TaskSnap]) -> bool {
    snaps.iter().any(|snap| {
        snap.plan_item_id.as_deref() == Some(item_id)
            && !snap.streaming
            && !snap.failed
            && !snap.queued
    })
}

fn carried_item(job: &Job, old: &PlanItem) -> PlanItem {
    let mut context = old.context.clone();
    if !context.contains("Carried over") {
        if !context.is_empty() {
            context.push(' ');
        }
        context.push_str("Carried over from the previous plan.");
    }
    draft_item(
        job,
        &old.title,
        &context,
        &old.done_when,
        "carried",
        old.agent_route.clone(),
        old.workspace_id.clone(),
        &old.workspace_name,
    )
}

fn template_item(job: &Job, title: &str) -> PlanItem {
    draft_item(
        job,
        title,
        "Standing task from the daily plan.",
        "The standing task is done.",
        "template",
        None,
        None,
        "",
    )
}

fn draft_item(
    job: &Job,
    title: &str,
    context: &str,
    done_when: &str,
    source: &str,
    agent_route: Option<String>,
    workspace_id: Option<String>,
    workspace_name: &str,
) -> PlanItem {
    let template = if job.template.trim().is_empty() {
        DEFAULT_TEMPLATE
    } else {
        job.template.as_str()
    };
    let body = template
        .replace("{task.title}", title)
        .replace("{task.context}", context)
        .replace("{task.done_when}", done_when);
    PlanItem {
        id: String::new(),
        title: title.to_string(),
        context: context.to_string(),
        done_when: done_when.to_string(),
        source: source.to_string(),
        prompt: format!("{PREAMBLE}{body}"),
        agent_route,
        workspace_id,
        workspace_name: workspace_name.to_string(),
        selected: true,
        approved: false,
        skipped: false,
        started: false,
        session_id: None,
    }
}

fn diff_line(previous: Option<&PlanRun>, items: &[PlanItem]) -> String {
    let Some(previous) = previous else {
        return "No earlier plan to compare.".into();
    };
    let old: Vec<&str> = previous.items.iter().map(|item| item.title.as_str()).collect();
    let new: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
    let completed = old.iter().filter(|title| !new.contains(title)).count();
    let carried = new.iter().filter(|title| old.contains(title)).count();
    let added = new.iter().filter(|title| !old.contains(title)).count();
    let mut line = format!("Against the previous plan: completed {completed}, carried {carried}, added {added}.");
    if previous.resolution == "timeout_skip_merge" {
        line.push_str(" Previous plan timed out and was merged.");
    }
    line
}

fn pending_for(job_id: &str) -> Option<PlanRun> {
    load_plans()
        .into_iter()
        .find(|plan| plan.job_id == job_id && plan.status == "pending")
}

fn latest_plan(job_id: &str) -> Option<PlanRun> {
    load_plans()
        .into_iter()
        .rev()
        .find(|plan| plan.job_id == job_id)
}

fn fresh_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{now}-{n}")
}

fn load_jobs() -> Vec<Job> {
    let Ok(text) = std::fs::read_to_string(jobs_path()) else {
        return Vec::new();
    };
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        return Vec::new();
    };
    values.into_iter().map(parse_job).collect()
}

fn parse_job(value: serde_json::Value) -> Job {
    let due_at = value["due_at"]
        .as_u64()
        .or_else(|| value["due_at"].as_str().and_then(|text| text.parse().ok()))
        .unwrap_or(0);
    let kind = value["kind"].as_str().unwrap_or("once").to_string();
    let suggest = value["suggest"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Job {
        id: value["id"].as_str().unwrap_or("").to_string(),
        kind,
        title: value["title"].as_str().unwrap_or("Scheduled task").to_string(),
        prompt: value["prompt"].as_str().unwrap_or("").to_string(),
        due_at,
        daily_minutes: value["daily_minutes"].as_u64().unwrap_or(8 * 60 + 30) as u32,
        interval_hours: value["interval_hours"].as_u64().unwrap_or(1) as u32,
        periodic: serde_json::from_value(value["periodic"].clone()).ok(),
        next_run_at: value["next_run_at"].as_u64().unwrap_or(0),
        force: value["force"].as_bool().unwrap_or(false),
        deadline_minutes: value["deadline_minutes"].as_u64().unwrap_or(45) as u32,
        template: value["template"]
            .as_str()
            .unwrap_or(DEFAULT_TEMPLATE)
            .to_string(),
        suggest,
    }
}

fn save_jobs(jobs: &[Job]) {
    let values: Vec<serde_json::Value> = jobs.iter().map(job_json).collect();
    write_atomic(&jobs_path(), &values);
}

fn job_json(job: &Job) -> serde_json::Value {
    if job.kind == "plan" {
        serde_json::json!({
            "id": job.id,
            "kind": "plan",
            "title": job.title,
            "prompt": job.prompt,
            "daily_minutes": job.daily_minutes,
            "next_run_at": job.next_run_at,
            "force": job.force,
            "deadline_minutes": job.deadline_minutes,
            "template": job.template,
            "suggest": job.suggest,
        })
    } else if job.kind == "periodic" {
        serde_json::json!({
            "id": job.id,
            "kind": "periodic",
            "title": job.title,
            "prompt": job.prompt,
            "periodic": job.periodic,
            "next_run_at": job.next_run_at,
        })
    } else if job.kind == "daily" || job.kind == "interval" {
        serde_json::json!({
            "id": job.id,
            "kind": job.kind,
            "title": job.title,
            "prompt": job.prompt,
            "daily_minutes": job.daily_minutes,
            "interval_hours": job.interval_hours,
            "next_run_at": job.next_run_at,
        })
    } else {
        serde_json::json!({
            "title": job.title,
            "prompt": job.prompt,
            "due_at": job.due_at,
        })
    }
}

fn load_plans() -> Vec<PlanRun> {
    let Ok(text) = std::fs::read_to_string(plans_path()) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_plans(plans: &[PlanRun]) {
    // `&plans` keeps the slice behind a sized reference. `plans` alone is
    // `&[PlanRun]`, which the `impl Serialize` bound treats as unsized.
    write_atomic(&plans_path(), &plans);
}

fn write_atomic(path: &Path, value: &impl Serialize) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let bytes = serde_json::to_vec_pretty(value).unwrap_or_else(|_| b"[]".to_vec());
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, bytes).is_err() {
        return;
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evobuddy-schedule-{name}-{}-{}",
            std::process::id(),
            fresh_id("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn snap(title: &str, at: u64, failed: bool) -> TaskSnap {
        TaskSnap {
            title: title.into(),
            workspace_id: None,
            workspace_name: String::new(),
            agent_route: None,
            updated_at: Some(at),
            streaming: false,
            failed,
            queued: false,
            plan_item_id: None,
            excerpt: format!("notes for {title}"),
        }
    }

    #[test]
    fn next_daily_run_rolls_to_tomorrow_once_the_slot_has_passed() {
        let day = 10_000 * 86_400;
        let at_0829 = day + 8 * 3600 + 29 * 60;
        assert_eq!(next_daily_run(at_0829, 510, 0), day + 8 * 3600 + 30 * 60);
        let at_0830 = day + 8 * 3600 + 30 * 60;
        assert_eq!(next_daily_run(at_0830, 510, 0), day + 86_400 + 510 * 60);
    }

    #[test]
    fn a_one_shot_job_is_claimed_once_and_a_plan_job_stays() {
        let dir = temp_dir("once");
        let day = 10_000 * 86_400;
        let jobs = serde_json::json!([
            { "title": "Hello", "prompt": "Say hi", "due_at": 1 },
            {
                "id": "job-1",
                "kind": "plan",
                "title": "Daily",
                "daily_minutes": 510,
                "next_run_at": day + 86_400,
                "suggest": []
            }
        ]);
        std::fs::write(dir.join("scheduled.json"), jobs.to_string()).unwrap();
        let events = tick_at(&dir, 100, 0, &[]);
        assert!(matches!(&events[0], Tick::RunPrompt { title, prompt } if title == "Hello" && prompt == "Say hi"));
        let again = tick_at(&dir, 100, 0, &[]);
        assert!(again.is_empty(), "the one-shot prompt must not repeat");
        let left: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scheduled.json")).unwrap())
                .unwrap();
        assert_eq!(left.as_array().unwrap().len(), 1);
        assert_eq!(left[0]["kind"], "plan");
    }

    #[test]
    fn a_due_plan_is_written_once_and_then_waits_for_tomorrow() {
        let dir = temp_dir("plan");
        let day = 10_000 * 86_400;
        let slot = day + 510 * 60;
        let jobs = serde_json::json!([{
            "id": "job-1",
            "kind": "plan",
            "title": "Daily",
            "daily_minutes": 510,
            "next_run_at": slot,
            "deadline_minutes": 45,
            "suggest": ["Write the changelog"]
        }]);
        std::fs::write(dir.join("scheduled.json"), jobs.to_string()).unwrap();
        let snaps = vec![
            snap("Fix login", slot - 3600, true),
            snap("Docs", slot - 1800, false),
        ];
        let events = tick_at(&dir, slot + 60, 0, &snaps);
        assert!(matches!(&events[0], Tick::PlanReady { items, .. } if *items == 2));
        let quiet = tick_at(&dir, slot + 120, 0, &snaps);
        assert!(quiet.is_empty());
        let _guard = DirGuard::push(dir.clone());
        let plan = plans().pop().unwrap();
        assert_eq!(plan.status, "pending");
        assert_eq!(plan.items[0].title, "Fix login");
        assert_eq!(plan.items[1].title, "Write the changelog");
        assert!(plan.items[0].selected);
        assert!(plan.summary.contains("Completed 1"));
        assert!(plan.diff.contains("No earlier plan"));
    }

    #[test]
    fn approving_twice_does_not_queue_the_skipped_rows_again() {
        let dir = temp_dir("approve");
        let _guard = DirGuard::push(dir.clone());
        let day = 10_000 * 86_400;
        let slot = day + 510 * 60;
        std::fs::write(
            dir.join("scheduled.json"),
            serde_json::json!([{
                "id": "job-1",
                "kind": "plan",
                "title": "Daily",
                "daily_minutes": 510,
                "next_run_at": slot,
                "suggest": ["Changelog"]
            }])
            .to_string(),
        )
        .unwrap();
        tick_at(&dir, slot, 0, &[snap("Fix login", slot - 10, true)]);
        let plan_id = plans()[0].id.clone();
        let second = plans()[0].items[1].id.clone();
        set_selected(&plan_id, &second, false);
        assert_eq!(decide(&plan_id, DecisionKind::Selected), 1);
        assert_eq!(decide(&plan_id, DecisionKind::All), 0);
        let plan = plans().pop().unwrap();
        assert_eq!(plan.status, "partial");
        assert!(plan.items[0].approved);
        assert!(plan.items[1].skipped);
        assert_eq!(dispatch_queue().len(), 1);
        mark_started(&plan.items[0].id);
        assert!(dispatch_queue().is_empty());
    }

    #[test]
    fn a_timeout_is_visible_and_its_items_merge_into_the_next_plan() {
        let dir = temp_dir("timeout");
        let day = 10_000 * 86_400;
        let slot = day + 510 * 60;
        std::fs::write(
            dir.join("scheduled.json"),
            serde_json::json!([{
                "id": "job-1",
                "kind": "plan",
                "title": "Daily",
                "daily_minutes": 510,
                "next_run_at": slot,
                "deadline_minutes": 45,
                "suggest": ["Changelog"]
            }])
            .to_string(),
        )
        .unwrap();
        tick_at(&dir, slot, 0, &[]);
        let expired = tick_at(&dir, slot + 45 * 60, 0, &[]);
        assert!(matches!(&expired[0], Tick::PlanExpired { .. }));
        let _guard = DirGuard::push(dir.clone());
        let skipped = plans().pop().unwrap();
        assert_eq!(skipped.resolution, "timeout_skip_merge");
        assert!(skipped.items.iter().all(|item| item.skipped));
        // Tomorrow's slot.
        let next = next_daily_run(slot + 45 * 60, 510, 0);
        let ready = tick_at(&dir, next, 0, &[]);
        assert!(matches!(&ready[0], Tick::PlanReady { items: 1, .. }));
        let plan = plans().into_iter().last().unwrap();
        assert_eq!(plan.items[0].title, "Changelog");
        assert_eq!(plan.items[0].source, "carried");
        assert!(plan.diff.contains("carried 1"));
        assert!(plan.diff.contains("timed out"));
    }

    #[test]
    fn a_finished_carried_item_is_not_merged_again() {
        let dir = temp_dir("done");
        let day = 10_000 * 86_400;
        let slot = day + 510 * 60;
        std::fs::write(
            dir.join("scheduled.json"),
            serde_json::json!([{
                "id": "job-1",
                "kind": "plan",
                "title": "Daily",
                "daily_minutes": 510,
                "next_run_at": slot,
                "deadline_minutes": 45,
                "suggest": []
            }])
            .to_string(),
        )
        .unwrap();
        tick_at(&dir, slot, 0, &[snap("Fix login", slot - 10, true)]);
        let _guard = DirGuard::push(dir.clone());
        let plan_id = plans()[0].id.clone();
        decide(&plan_id, DecisionKind::All);
        let item_id = plans()[0].items[0].id.clone();
        mark_started(&item_id);
        let next = next_daily_run(slot, 510, 0);
        let mut done = snap("Fix login", next - 60, false);
        done.plan_item_id = Some(item_id);
        let events = tick_at(&dir, next, 0, &[done]);
        assert!(matches!(&events[0], Tick::PlanReady { items: 0, .. }));
    }


    #[test]
    fn a_daily_prompt_job_fires_once_and_waits_for_tomorrow() {
        let dir = temp_dir("daily");
        let day = 10_000 * 86_400;
        let slot = day + 9 * 3600;
        let jobs = serde_json::json!([{
            "id": "job-d1",
            "kind": "daily",
            "title": "Standup",
            "prompt": "Post the standup",
            "daily_minutes": 540,
            "next_run_at": slot,
        }]);
        std::fs::write(dir.join("scheduled.json"), jobs.to_string()).unwrap();
        let events = tick_at(&dir, slot + 30, 0, &[]);
        assert!(matches!(&events[0], Tick::RunPrompt { title, prompt } if title == "Standup" && prompt == "Post the standup"));
        let again = tick_at(&dir, slot + 60, 0, &[]);
        assert!(again.is_empty(), "a daily job must not fire twice a day");
        let left: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scheduled.json")).unwrap())
                .unwrap();
        assert_eq!(left[0]["kind"], "daily");
        // The next 09:00 in the machine's local time (the tick offsets by it).
        let offset = local_offset_secs(slot + 30);
        assert_eq!(
            left[0]["next_run_at"],
            next_daily_run(slot + 30, 540, offset)
        );
    }

    #[test]
    fn an_interval_prompt_job_fires_repeatedly() {
        let dir = temp_dir("interval");
        let start = 10_000 * 86_400 + 3600;
        let jobs = serde_json::json!([{
            "id": "job-i1",
            "kind": "interval",
            "title": "Stretch",
            "prompt": "Time to stretch",
            "interval_hours": 2,
            "next_run_at": start,
        }]);
        std::fs::write(dir.join("scheduled.json"), jobs.to_string()).unwrap();
        let events = tick_at(&dir, start, 0, &[]);
        assert!(matches!(&events[0], Tick::RunPrompt { title, .. } if title == "Stretch"));
        let left: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scheduled.json")).unwrap())
                .unwrap();
        assert_eq!(left[0]["next_run_at"], start + 7200);
        let events = tick_at(&dir, start + 7200, 0, &[]);
        assert!(matches!(&events[0], Tick::RunPrompt { .. }));
    }

    #[test]
    fn once_at_schedules_the_next_occurrence_of_the_time() {
        let dir = temp_dir("once-at");
        let _guard = DirGuard::push(dir.clone());
        let now = unix_now();
        add_once_at("Evening review", "Review the day", 21 * 60 + 30);
        let jobs: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scheduled.json")).unwrap())
                .unwrap();
        let due = jobs[0]["due_at"].as_u64().unwrap();
        assert!(due > now, "the slot must be in the future");
        let offset = local_offset_secs(now);
        assert_eq!(due, next_daily_run(now, 21 * 60 + 30, offset));
    }

    #[test]
    fn plan_full_persists_the_template_and_deadline() {
        let dir = temp_dir("plan-full");
        let _guard = DirGuard::push(dir.clone());
        add_plan_full(
            "Morning plan",
            510,
            &["Changelog".to_string()],
            "Do \"{task.title}\" carefully.",
            90,
        );
        let jobs: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scheduled.json")).unwrap())
                .unwrap();
        assert_eq!(jobs[0]["deadline_minutes"], 90);
        assert_eq!(jobs[0]["template"], "Do \"{task.title}\" carefully.");
        // A reparse keeps both fields.
        let job = parse_job(jobs[0].clone());
        assert_eq!(job.deadline_minutes, 90);
        assert_eq!(job.template, "Do \"{task.title}\" carefully.");
    }

    #[test]
    fn civil_dates_and_weekdays_match_the_gregorian_calendar() {
        // 1970-01-01 was a Thursday (Mon=0 → 3).
        assert_eq!(weekday_monday0(0), 3);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-10-31 is a Saturday: 20757 days after the epoch.
        // Day 10_000 is Monday 1997-05-19.
        let (y, m, d) = civil_from_days(20_757);
        assert_eq!((y, m, d), (2026, 10, 31));
        assert_eq!(weekday_monday0(20_757), 5);
        assert_eq!(civil_from_days(10_000), (1997, 5, 19));
    }

    #[test]
    fn a_weekly_schedule_picks_the_next_matching_weekday_at_the_time() {
        // Day 10_000 local is a Monday; 09:00 is day + 32_400.
        let monday = 10_000 * 86_400;
        let p = Periodic {
            freq: "week".into(),
            minutes: 540,
            byday: vec![0, 2], // MO, WE
            bymonthday: vec![],
            bymonth: 0,
            anchor_week: 0,
        };
        // 08:00 Monday: today matches → today 09:00.
        assert_eq!(
            next_periodic_run(monday + 8 * 3600, &p, 0),
            Some(monday + 9 * 3600)
        );
        // 10:00 Monday: the next match is Wednesday 09:00.
        assert_eq!(
            next_periodic_run(monday + 10 * 3600, &p, 0),
            Some(monday + 2 * 86_400 + 9 * 3600)
        );
        // Friday 10:00: wraps to Monday next week (+7 days).
        assert_eq!(
            next_periodic_run(monday + 4 * 86_400 + 10 * 3600, &p, 0),
            Some(monday + 7 * 86_400 + 9 * 3600)
        );
    }

    #[test]
    fn a_biweekly_schedule_keeps_its_creation_parity() {
        let monday = 10_000 * 86_400;
        let anchor = (monday / 86_400 / 7) as u64;
        let p = Periodic {
            freq: "biweek".into(),
            minutes: 540,
            byday: vec![1], // TU
            bymonthday: vec![],
            bymonth: 0,
            anchor_week: anchor,
        };
        // This week's Tuesday fires, then the Tuesday two weeks later.
        assert_eq!(
            next_periodic_run(monday + 10 * 3600, &p, 0),
            Some(monday + 86_400 + 9 * 3600)
        );
        let after = monday + 86_400 + 10 * 3600;
        assert_eq!(
            next_periodic_run(after, &p, 0),
            Some(monday + (1 + 14) * 86_400 + 9 * 3600)
        );
    }

    #[test]
    fn monthly_and_yearly_schedules_land_on_the_requested_days() {
        // Day 20_757 is 2026-10-31 (Saturday).
        let sat = 20_757 * 86_400;
        let monthly = Periodic {
            freq: "month".into(),
            minutes: 600,
            byday: vec![],
            bymonthday: vec![15],
            bymonth: 0,
            anchor_week: 0,
        };
        // 2026-10-31 10:00 → 2026-11-15 10:00 (15 days later).
        assert_eq!(
            next_periodic_run(sat + 10 * 3600, &monthly, 0),
            Some(sat + 15 * 86_400 + 10 * 3600)
        );
        let yearly = Periodic {
            freq: "year".into(),
            minutes: 600,
            byday: vec![],
            bymonthday: vec![1],
            bymonth: 1,
            anchor_week: 0,
        };
        // → 2027-01-01 10:00.
        assert_eq!(
            next_periodic_run(sat + 10 * 3600, &yearly, 0),
            Some((20_757 + 62) * 86_400 + 10 * 3600)
        );
    }

    #[test]
    fn the_state_lock_nests_on_a_thread_and_reads_busy_from_another() {
        let dir = temp_dir("lock");
        let _guard = DirGuard::push(dir.clone());
        let outer = try_lock().expect("first acquire on a quiet directory");
        assert!(
            try_lock().is_some(),
            "a second acquire on the same thread nests instead of failing"
        );
        let held = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _guard = DirGuard::push(dir);
                try_lock().is_none()
            })
            .join()
            .unwrap()
        };
        assert!(held, "another thread must see the lock as busy");
        drop(outer);
        let free = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _guard = DirGuard::push(dir);
                try_lock().is_some()
            })
            .join()
            .unwrap()
        };
        assert!(free, "the lock must be takeable after release");
    }

    #[test]
    fn a_headless_tick_writes_the_plan_and_leaves_the_one_shot() {
        let dir = temp_dir("headless");
        let day = 10_000 * 86_400;
        let slot = day + 510 * 60;
        std::fs::write(
            dir.join("scheduled.json"),
            serde_json::json!([
                { "title": "Ping", "prompt": "Say hi", "due_at": 1 },
                {
                    "id": "job-1",
                    "kind": "plan",
                    "title": "Daily",
                    "daily_minutes": 510,
                    "next_run_at": slot,
                    "suggest": ["Changelog"]
                }
            ])
            .to_string(),
        )
        .unwrap();
        let events = tick_plans_at(&dir, slot, 0, &[]);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, Tick::RunPrompt { .. })),
            "a headless tick must not consume a one-shot"
        );
        assert!(matches!(&events[0], Tick::PlanReady { items: 1, .. }));
        let left: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scheduled.json")).unwrap())
                .unwrap();
        let jobs = left.as_array().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().any(|job| job["prompt"] == "Say hi"));
        let _guard = DirGuard::push(dir.clone());
        let plan = plans().pop().unwrap();
        assert_eq!(plan.status, "pending");
        assert!(dispatch_queue().is_empty());
        drop(_guard);
        let claimed = tick_at(&dir, slot + 10, 0, &[]);
        assert!(matches!(&claimed[0], Tick::RunPrompt { prompt, .. } if prompt == "Say hi"));
    }
}
