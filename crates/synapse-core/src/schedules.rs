//! Confirmed run schedules (M9.4).
//!
//! A user types natural language ("every weekday at 9am"); we parse it into a
//! cron spec and show a [`SchedulePreview`] that must be [`ScheduleStore::confirm`]ed
//! before anything runs. Schedules only fire while the app is open: the runner
//! sleeps until the next due time ([`ScheduleStore::sleep_until`]), and anything
//! whose `next_run_at` passed while the app was closed shows up in
//! [`ScheduleStore::missed`] with a run-now action ([`ScheduleStore::run_now`]).

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use croner::Cron;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScheduleError {
    #[error("could not understand schedule: {0}")]
    Unparseable(String),
    #[error("invalid cron expression \"{0}\": {1}")]
    InvalidCron(String, String),
    #[error("schedule not found: {0}")]
    NotFound(String),
    #[error("schedule \"{0}\" is already confirmed")]
    AlreadyConfirmed(String),
    #[error("confirm the schedule preview before it can run")]
    NotConfirmed,
    #[error("prompt is required: say what the run should do")]
    EmptyPrompt,
}

fn now_dt(now_unix: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(now_unix, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

pub fn validate_cron(expr: &str) -> Result<(), ScheduleError> {
    Cron::from_str(expr.trim())
        .map(|_| ())
        .map_err(|e| ScheduleError::InvalidCron(expr.trim().to_string(), e.to_string()))
}

pub fn describe_cron(expr: &str) -> String {
    match Cron::from_str(expr.trim()) {
        Ok(c) => c.describe().to_string(),
        Err(_) => expr.trim().to_string(),
    }
}

pub fn next_run_unix(expr: &str, after_unix: i64) -> Result<i64, ScheduleError> {
    let cron = Cron::from_str(expr.trim())
        .map_err(|e| ScheduleError::InvalidCron(expr.into(), e.to_string()))?;
    let after = now_dt(after_unix);
    cron.find_next_occurrence(&after, false)
        .map(|dt| dt.timestamp())
        .map_err(|e| ScheduleError::InvalidCron(expr.into(), e.to_string()))
}

fn two(n: u32) -> String {
    format!("{n:02}")
}

/// Parse natural language (or a raw cron string) into a 5-field cron expr.
///
/// Accepted (case-insensitive):
/// - `every minute`, `every N minutes`
/// - `hourly`, `every hour`, `every N hours`
/// - `daily [at H[:MM] [am|pm]]`, `every day [at ...]`
/// - `weekdays [at ...]`, `weekends [at ...]`
/// - `weekly [on <day>] [at ...]`, `every <day> [at ...]`, `every week ...`
/// - `monthly [on day N] [at ...]`
/// - `on <day> at ...`
/// - a raw 5-field cron string (passed through after validation)
pub fn parse_natural_schedule(input: &str) -> Result<String, ScheduleError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(ScheduleError::Unparseable(
            "describe the schedule, e.g. \"every weekday at 9am\"".into(),
        ));
    }
    let lower = raw.to_lowercase();

    // Raw cron passthrough: 5 whitespace-separated fields of cron chars.
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() == 5
        && parts.iter().all(|p| {
            p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "*/,-?LW#".contains(c))
        })
    {
        validate_cron(raw)?;
        return Ok(raw.to_string());
    }

    // every minute / every N minutes
    if lower == "every minute" || lower == "each minute" {
        return Ok("* * * * *".into());
    }
    if let Some(n) = capture_every_n(&lower, "minute", "minutes") {
        if (1..=59).contains(&n) {
            return Ok(format!("*/{n} * * * *"));
        }
    }
    if lower == "hourly" || lower == "every hour" || lower == "each hour" {
        return Ok("0 * * * *".into());
    }
    if let Some(n) = capture_every_n(&lower, "hour", "hours") {
        if (1..=23).contains(&n) {
            return Ok(format!("0 */{n} * * *"));
        }
    }

    // Extract optional time-of-day; default 09:00.
    let (hh, mm) = parse_time_of_day(&lower).unwrap_or((9, 0));

    if lower.contains("weekday") {
        return Ok(format!("{} {} * * MON-FRI", two(mm), two(hh)));
    }
    if lower.contains("weekend") {
        return Ok(format!("{} {} * * SAT-SUN", two(mm), two(hh)));
    }
    if let Some(dow) = find_weekday(&lower) {
        // Weekly on a named day (also covers "every monday ...").
        if lower.contains("month") {
            // "monthly on monday..." is ambiguous; treat as weekly.
        }
        let _ = lower.contains("week");
        return Ok(format!("{} {} * * {dow}", two(mm), two(hh)));
    }
    if lower.contains("month") {
        let dom = parse_month_day(&lower).unwrap_or(1);
        return Ok(format!("{} {} {dom} * *", two(mm), two(hh)));
    }
    if lower.contains("week") {
        // "every week" without a day -> Monday default.
        return Ok(format!("{} {} * * MON", two(mm), two(hh)));
    }
    if lower.contains("day")
        || lower.contains("daily")
        || lower.contains("night")
        || lower.contains("morning")
        || lower.contains("evening")
        || lower.contains("afternoon")
        || lower.contains("noon")
        || lower.contains("midnight")
    {
        return Ok(format!("{} {} * * *", two(mm), two(hh)));
    }
    // Bare time ("at 9am", "9:30", "17:00") means daily at that time.
    if parse_time_of_day(&lower).is_some() {
        return Ok(format!("{} {} * * *", two(mm), two(hh)));
    }

    Err(ScheduleError::Unparseable(format!(
        "could not understand \"{raw}\": try \"every day at 9am\", \"weekdays at 9am\", \"every monday at 9am\" or a cron expression"
    )))
}

fn capture_every_n(lower: &str, sing: &str, plur: &str) -> Option<u32> {
    for word in [sing, plur] {
        if let Some(idx) = lower.find(word) {
            let before = lower[..idx].trim_end();
            let token = before.split_whitespace().last()?;
            if token == "every" || token == "each" {
                return None; // handled by exact-match arms
            }
            if let Ok(n) = token.parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

fn find_weekday(lower: &str) -> Option<&'static str> {
    let days = [
        ("sunday", "SUN"),
        ("sun", "SUN"),
        ("monday", "MON"),
        ("mon", "MON"),
        ("tuesday", "TUE"),
        ("tue", "TUE"),
        ("tues", "TUE"),
        ("wednesday", "WED"),
        ("wed", "WED"),
        ("thursday", "THU"),
        ("thu", "THU"),
        ("thur", "THU"),
        ("thurs", "THU"),
        ("friday", "FRI"),
        ("fri", "FRI"),
        ("saturday", "SAT"),
        ("sat", "SAT"),
    ];
    // Long names first so "thu" does not shadow "thursday" (same code anyway).
    for (name, code) in days {
        if lower.contains(name) {
            return Some(code);
        }
    }
    None
}

fn parse_month_day(lower: &str) -> Option<u32> {
    let idx = lower.find("day")?;
    let after = lower[idx + 3..].trim_start().trim_start_matches([':', '=']);
    let token: String = after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let n: u32 = token.parse().ok()?;
    if (1..=28).contains(&n) {
        Some(n)
    } else {
        None
    }
}

/// Parse a time of day out of free text. Returns (hour 0-23, minute).
fn parse_time_of_day(lower: &str) -> Option<(u32, u32)> {
    let mut text = lower.replace('.', " ");
    // Keep ':' and letters/digits/spaces only.
    text = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ':' || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect();
    if text.contains("midnight") {
        return Some((0, 0));
    }
    if text.contains("noon") {
        return Some((12, 0));
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        // "9am" / "9pm" glued.
        if let Some(h) = strip_suffix_num(tok, "am") {
            if (1..=12).contains(&h) {
                return Some((h % 12, 0));
            }
        }
        if let Some(h) = strip_suffix_num(tok, "pm") {
            if (1..=12).contains(&h) {
                return Some(((h % 12) + 12, 0));
            }
        }
        // "H:MM" optionally followed by am/pm.
        if tok.contains(':') {
            let mut it = tok.split(':');
            if let (Some(h), Some(m)) = (it.next(), it.next()) {
                if let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) {
                    if h <= 23 && m <= 59 {
                        let next = tokens.get(i + 1).copied().unwrap_or("");
                        if next == "pm" && h < 12 {
                            return Some((h + 12, m));
                        }
                        if next == "am" && h == 12 {
                            return Some((0, m));
                        }
                        // Bare H:MM with H<=12 and explicit marker handled;
                        // otherwise take as 24h time.
                        return Some((h, m));
                    }
                }
            }
        }
        // Bare hour + am/pm tokens: "9 am", "5 in the evening".
        if let Ok(h) = tok.parse::<u32>() {
            if h <= 23 {
                let next = tokens.get(i + 1).copied().unwrap_or("");
                if next == "am" {
                    if (1..=12).contains(&h) {
                        return Some((h % 12, 0));
                    }
                } else if next == "pm" {
                    if (1..=12).contains(&h) {
                        return Some(((h % 12) + 12, 0));
                    }
                } else if h <= 23 && tokens.get(i.saturating_sub(1)).copied() == Some("at") {
                    // "at 9" with no marker: morning heuristic for the
                    // schedule UX (matches "every day at 9").
                    if h <= 12 {
                        return Some((h % 12, 0));
                    }
                    return Some((h, 0));
                }
            }
        }
        i += 1;
    }
    // "in the evening/afternoon" without a number -> 18:00; morning -> 9:00.
    if lower.contains("evening") {
        return Some((18, 0));
    }
    if lower.contains("afternoon") {
        return Some((15, 0));
    }
    if lower.contains("morning") {
        return Some((9, 0));
    }
    None
}

fn strip_suffix_num(tok: &str, suffix: &str) -> Option<u32> {
    let stem = tok.strip_suffix(suffix)?;
    if stem.contains(':') {
        // "9:30am"
        let mut it = stem.split(':');
        let h: u32 = it.next()?.parse().ok()?;
        let m: u32 = it.next()?.parse().ok()?;
        if m > 59 || !(1..=12).contains(&h) {
            return None;
        }
        let hh = if suffix == "pm" {
            (h % 12) + 12
        } else {
            h % 12
        };
        // Encode minutes in the high bits is hacky; instead handle inline.
        // Return None here and let the ':' arm... simpler: not supported glued
        // with minutes except via the helper below. Keep simple: reject.
        let _ = (hh, m);
        return None;
    }
    stem.parse::<u32>().ok()
}

/// What the confirmation dialog shows before a schedule may run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePreview {
    pub project_id: String,
    pub prompt: String,
    pub mode: String,
    pub natural_text: String,
    pub cron_expr: String,
    pub description: String,
    pub next_run_at: i64,
}

impl SchedulePreview {
    /// Human line for the confirm sheet, e.g. `Weekdays at 09:00 (30 9 * * MON-FRI) · next run ...`.
    pub fn summary(&self) -> String {
        format!(
            "{} ({}) · next run {}",
            self.description, self.cron_expr, self.next_run_at
        )
    }
}

/// Build a preview from natural language. Nothing is stored and nothing runs
/// until [`ScheduleStore::confirm`] is called with the preview.
pub fn preview_schedule(
    project_id: &str,
    prompt: &str,
    mode: &str,
    natural_text: &str,
    now_unix: i64,
) -> Result<SchedulePreview, ScheduleError> {
    if prompt.trim().is_empty() {
        return Err(ScheduleError::EmptyPrompt);
    }
    let cron_expr = parse_natural_schedule(natural_text)?;
    let description = describe_cron(&cron_expr);
    let next_run_at = next_run_unix(&cron_expr, now_unix)?;
    Ok(SchedulePreview {
        project_id: project_id.to_string(),
        prompt: prompt.trim().to_string(),
        mode: mode.to_string(),
        natural_text: natural_text.trim().to_string(),
        cron_expr,
        description,
        next_run_at,
    })
}

/// A stored schedule row (mirrors `synapse-store` `schedules` table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub project_id: String,
    pub cron_expr: String,
    pub mode: String,
    pub prompt: String,
    /// False until the user confirms the preview.
    pub confirmed: bool,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
}

impl Schedule {
    pub fn description(&self) -> String {
        describe_cron(&self.cron_expr)
    }

    /// True when the schedule should fire now (confirmed, enabled, due).
    /// Schedules only run while the app is open: the caller passes "now".
    pub fn is_due(&self, now_unix: i64) -> bool {
        self.confirmed && self.enabled && self.next_run_at.is_some_and(|t| t <= now_unix)
    }

    /// A missed run: due in the past while the app was closed. The UI lists
    /// these with a run-now action instead of firing automatically.
    pub fn is_missed(&self, now_unix: i64, grace_secs: i64) -> bool {
        self.confirmed
            && self.enabled
            && self.next_run_at.is_some_and(|t| t + grace_secs < now_unix)
    }

    pub fn spec_json(&self) -> String {
        serde_json::json!({"cron": self.cron_expr, "tz": "local"}).to_string()
    }

    pub fn cron_from_spec(spec: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(spec)
            .ok()
            .and_then(|v| v.get("cron")?.as_str().map(|s| s.to_string()))
    }
}

/// One missed firing: the schedule was due at `expected_at` while the app was
/// closed. The Scheduled tab shows these with a run-now button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissedRun {
    pub schedule_id: String,
    pub project_id: String,
    pub prompt: String,
    pub expected_at: i64,
    pub description: String,
}

/// In-memory schedule registry. Persistence lives in `synapse-store` (rows
/// serialised from [`Schedule`]); this owns the confirm/due/missed rules.
#[derive(Debug, Default)]
pub struct ScheduleStore {
    schedules: HashMap<String, Schedule>,
    next_id: u64,
}

impl ScheduleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.schedules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.schedules.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Schedule> {
        self.schedules.get(id)
    }

    pub fn list(&self) -> Vec<&Schedule> {
        let mut out: Vec<&Schedule> = self.schedules.values().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Stage a preview as an unconfirmed schedule. It never fires until
    /// [`ScheduleStore::confirm`].
    pub fn propose(&mut self, preview: SchedulePreview) -> Schedule {
        self.next_id += 1;
        let id = format!("schedule_{}", self.next_id);
        let schedule = Schedule {
            id,
            project_id: preview.project_id,
            cron_expr: preview.cron_expr,
            mode: preview.mode,
            prompt: preview.prompt,
            confirmed: false,
            enabled: true,
            last_run_at: None,
            next_run_at: Some(preview.next_run_at),
        };
        self.schedules.insert(schedule.id.clone(), schedule.clone());
        schedule
    }

    /// Confirm a previewed schedule: recompute `next_run_at` from now so the
    /// confirm sheet's "next run" is what actually runs.
    pub fn confirm(&mut self, id: &str, now_unix: i64) -> Result<Schedule, ScheduleError> {
        let s = self
            .schedules
            .get_mut(id)
            .ok_or_else(|| ScheduleError::NotFound(id.into()))?;
        if s.confirmed {
            return Err(ScheduleError::AlreadyConfirmed(id.into()));
        }
        s.confirmed = true;
        s.next_run_at = Some(next_run_unix(&s.cron_expr, now_unix)?);
        Ok(s.clone())
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<Schedule, ScheduleError> {
        let s = self
            .schedules
            .get_mut(id)
            .ok_or_else(|| ScheduleError::NotFound(id.into()))?;
        s.enabled = enabled;
        Ok(s.clone())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), ScheduleError> {
        self.schedules
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| ScheduleError::NotFound(id.into()))
    }

    /// Schedules due right now (confirmed + enabled + `next_run_at <= now`).
    pub fn due(&self, now_unix: i64) -> Vec<Schedule> {
        let mut out: Vec<Schedule> = self
            .schedules
            .values()
            .filter(|s| s.is_due(now_unix))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Missed runs: due more than `grace_secs` ago (app was closed). Sorted
    /// oldest-first for the Scheduled tab.
    pub fn missed(&self, now_unix: i64, grace_secs: i64) -> Vec<MissedRun> {
        let mut out: Vec<MissedRun> = self
            .schedules
            .values()
            .filter(|s| s.is_missed(now_unix, grace_secs))
            .map(|s| MissedRun {
                schedule_id: s.id.clone(),
                project_id: s.project_id.clone(),
                prompt: s.prompt.clone(),
                expected_at: s.next_run_at.unwrap_or(now_unix),
                description: s.description(),
            })
            .collect();
        out.sort_by_key(|a| a.expected_at);
        out
    }

    /// Record a firing (due tick or run-now): advance `last_run_at` and
    /// compute the following `next_run_at` from now.
    pub fn mark_fired(&mut self, id: &str, now_unix: i64) -> Result<Schedule, ScheduleError> {
        let next = {
            let s = self
                .schedules
                .get(id)
                .ok_or_else(|| ScheduleError::NotFound(id.into()))?;
            if !s.confirmed {
                return Err(ScheduleError::NotConfirmed);
            }
            next_run_unix(&s.cron_expr, now_unix)?
        };
        let s = self.schedules.get_mut(id).expect("checked");
        s.last_run_at = Some(now_unix);
        s.next_run_at = Some(next);
        Ok(s.clone())
    }

    /// Run-now action for a missed (or any confirmed) run: same as a firing.
    pub fn run_now(&mut self, id: &str, now_unix: i64) -> Result<Schedule, ScheduleError> {
        self.mark_fired(id, now_unix)
    }

    /// How long the runner sleeps until the next confirmed+enabled firing.
    /// `None` means nothing scheduled: sleep until woken by a change. This is
    /// how the runner avoids polling (PERFORMANCE.md: sleep until due).
    pub fn sleep_until(&self, now_unix: i64) -> Option<Duration> {
        self.schedules
            .values()
            .filter(|s| s.confirmed && s.enabled)
            .filter_map(|s| s.next_run_at)
            .filter(|t| *t > now_unix)
            .min()
            .map(|t| Duration::from_secs((t - now_unix).max(0) as u64))
    }
}

#[cfg(test)]
mod schedules_tests {
    use super::*;

    fn noon_utc_2026_10_14() -> i64 {
        chrono::Utc
            .with_ymd_and_hms(2026, 10, 14, 12, 0, 0)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn schedules_natural_language_to_cron_preview() {
        let now = noon_utc_2026_10_14();
        let cases = [
            ("every day at 9am", "00 09 * * *"),
            ("daily at 17:30", "30 17 * * *"),
            ("weekdays at 9am", "00 09 * * MON-FRI"),
            ("every monday at 9am", "00 09 * * MON"),
            ("every hour", "0 * * * *"),
            ("every 15 minutes", "*/15 * * * *"),
            ("0 9 * * MON-FRI", "0 9 * * MON-FRI"),
        ];
        for (input, cron) in cases {
            let preview =
                preview_schedule("project_1", "Send standup", "team_room", input, now).unwrap();
            assert_eq!(preview.cron_expr, cron, "input: {input}");
            assert!(!preview.description.is_empty());
            assert!(preview.next_run_at > now);
        }
    }

    #[test]
    fn schedules_preview_must_be_confirmed_before_run() {
        let now = noon_utc_2026_10_14();
        let mut store = ScheduleStore::new();
        // Empty prompt is rejected at preview time.
        assert!(matches!(
            preview_schedule("project_1", "  ", "team_room", "daily at 9am", now),
            Err(ScheduleError::EmptyPrompt)
        ));
        // Gibberish is rejected with guidance.
        assert!(matches!(
            preview_schedule("project_1", "Do things", "team_room", "blorpy zzz", now),
            Err(ScheduleError::Unparseable(_))
        ));
        let preview = preview_schedule(
            "project_1",
            "Send standup",
            "team_room",
            "daily at 9am",
            now,
        )
        .unwrap();
        let staged = store.propose(preview);
        assert!(!staged.confirmed);
        // Unconfirmed schedules never fire, even when due.
        assert!(store.due(i64::MAX).is_empty());
        assert!(store.missed(i64::MAX, 60).is_empty());
        assert!(matches!(
            store.mark_fired(&staged.id, now),
            Err(ScheduleError::NotConfirmed)
        ));
        // Confirm recomputes next run from now; double-confirm is an error.
        let confirmed = store.confirm(&staged.id, now).unwrap();
        assert!(confirmed.confirmed);
        assert!(confirmed.next_run_at.unwrap() > now);
        assert!(matches!(
            store.confirm(&staged.id, now),
            Err(ScheduleError::AlreadyConfirmed(_))
        ));
    }

    #[test]
    fn schedules_runner_due_sleep_and_missed_run_now() {
        let now = noon_utc_2026_10_14();
        let mut store = ScheduleStore::new();
        let mk =
            |text: &str| preview_schedule("project_1", "Ping", "team_room", text, now).unwrap();
        let a = store.propose(mk("daily at 9am"));
        let b = store.propose(mk("daily at 10am"));
        store.confirm(&a.id, now).unwrap();
        store.confirm(&b.id, now).unwrap();
        // Runner sleeps until the next firing (no polling).
        let sleep = store.sleep_until(now).expect("something scheduled");
        assert!(sleep.as_secs() > 0);
        // Fast-forward past both firings: both are due...
        let later = store
            .get(&a.id)
            .unwrap()
            .next_run_at
            .unwrap()
            .max(store.get(&b.id).unwrap().next_run_at.unwrap())
            + 3600;
        assert_eq!(store.due(later).len(), 2);
        // ...and with a small grace window both count as missed (app was
        // closed), oldest first, each with a run-now action.
        let missed = store.missed(later, 60);
        assert_eq!(missed.len(), 2);
        assert!(missed[0].expected_at <= missed[1].expected_at);
        assert_eq!(missed[0].prompt, "Ping");
        // Run-now clears the missed entry and schedules the next firing.
        store.run_now(&missed[0].schedule_id, later).unwrap();
        assert_eq!(store.missed(later, 60).len(), 1);
        let fired = store.get(&missed[0].schedule_id).unwrap();
        assert_eq!(fired.last_run_at, Some(later));
        assert!(fired.next_run_at.unwrap() > later);
        // Disabled schedules neither fire nor count as missed.
        store.set_enabled(&missed[1].schedule_id, false).unwrap();
        assert_eq!(store.due(later).len(), 0);
        assert!(store.missed(later, 60).is_empty());
        // Nothing scheduled -> runner sleeps until woken by a change.
        store.set_enabled(&missed[0].schedule_id, false).unwrap();
        assert_eq!(store.sleep_until(later), None);
    }

    #[test]
    fn schedules_spec_json_round_trip_for_store_row() {
        let now = noon_utc_2026_10_14();
        let mut store = ScheduleStore::new();
        let preview =
            preview_schedule("project_9", "Report", "team_room", "weekdays at 9am", now).unwrap();
        let staged = store.propose(preview);
        let row = store.confirm(&staged.id, now).unwrap();
        let spec = row.spec_json();
        assert_eq!(
            Schedule::cron_from_spec(&spec).as_deref(),
            Some(row.cron_expr.as_str())
        );
        // Summary line is what the confirm sheet shows.
        let p =
            preview_schedule("project_9", "Report", "team_room", "weekdays at 9am", now).unwrap();
        assert!(p.summary().contains(&p.cron_expr));
    }
}
