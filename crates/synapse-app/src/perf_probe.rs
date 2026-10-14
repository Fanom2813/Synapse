//! Idle/tray performance probes (PERFORMANCE.md §3, MILESTONES.md M1.7).
//!
//! `synapse --perf-probe <idle|tray> --seconds N` runs the scenario with
//! mocks and prints one JSON [`ProbeReport`] line. Toolkit-free so the
//! budgets are testable headless; the GPUI shell only wires `needs_frame`
//! into its render loop.

use serde::{Deserialize, Serialize};

/// Hard caps from PERFORMANCE.md §1 (release build).
pub const IDLE_CPU_CAP_PERCENT: f64 = 0.5;
pub const IDLE_MEM_CAP_BYTES: u64 = 120 * 1024 * 1024;
pub const TRAY_CPU_CAP_PERCENT: f64 = 0.2;
pub const TRAY_MEM_CAP_BYTES: u64 = 60 * 1024 * 1024;

/// Seconds the window is given to settle before sampling starts.
pub const SETTLE_SECS: u64 = 5;

/// Scenario the probe runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeScenario {
    Idle,
    Tray,
}

impl ProbeScenario {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "idle" => Some(Self::Idle),
            "tray" => Some(Self::Tray),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Tray => "tray",
        }
    }

    pub fn cpu_cap_percent(self) -> f64 {
        match self {
            Self::Idle => IDLE_CPU_CAP_PERCENT,
            Self::Tray => TRAY_CPU_CAP_PERCENT,
        }
    }

    pub fn mem_cap_bytes(self) -> u64 {
        match self {
            Self::Idle => IDLE_MEM_CAP_BYTES,
            Self::Tray => TRAY_MEM_CAP_BYTES,
        }
    }
}

/// Parsed `--perf-probe` CLI args.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeArgs {
    pub scenario: ProbeScenario,
    pub seconds: u64,
}

impl ProbeArgs {
    /// Parse argv (without the binary name). Accepts
    /// `--perf-probe idle --seconds 30` and `--perf-probe=idle`.
    pub fn parse(argv: &[String]) -> Option<Self> {
        let mut scenario = None;
        let mut seconds = 30u64;
        let mut i = 0;
        while i < argv.len() {
            let a = argv[i].as_str();
            if let Some(v) = a.strip_prefix("--perf-probe=") {
                scenario = ProbeScenario::parse(v);
            } else if a == "--perf-probe" {
                i += 1;
                scenario = argv.get(i).and_then(|s| ProbeScenario::parse(s));
            } else if let Some(v) = a.strip_prefix("--seconds=") {
                seconds = v.parse().unwrap_or(30);
            } else if a == "--seconds" || a == "-s" {
                i += 1;
                if let Some(s) = argv.get(i) {
                    seconds = s.parse().unwrap_or(30);
                }
            }
            i += 1;
        }
        scenario.map(|scenario| Self { scenario, seconds })
    }
}

/// JSON line the probe prints (PERFORMANCE.md §3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeReport {
    pub scenario: ProbeScenario,
    pub seconds: u64,
    pub avg_cpu_percent: f64,
    pub avg_rss_bytes: u64,
    pub peak_rss_bytes: u64,
    /// macOS footprint; null elsewhere.
    pub footprint_bytes: Option<u64>,
    pub wakeups_per_sec: Option<u64>,
    pub frames_requested: u64,
    pub frames_per_second: f64,
    pub continuous_frames: bool,
    pub cpu_cap_percent: f64,
    pub mem_cap_bytes: u64,
    pub passed: bool,
    pub violations: Vec<String>,
}

impl ProbeReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Idle-frame policy: a settled window with nothing animating must not
/// request continuous frames (PERFORMANCE.md rule 2). The render loop
/// calls [`IdleFramePolicy::needs_frame`]; animations (avatar states,
/// streaming fade-ins) set `animating = true` while visible.
#[derive(Debug, Clone, Default)]
pub struct IdleFramePolicy {
    animating: bool,
    visible: bool,
    settled_secs: u64,
}

impl IdleFramePolicy {
    pub fn idle_settled() -> Self {
        Self {
            animating: false,
            visible: true,
            settled_secs: SETTLE_SECS,
        }
    }

    pub fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn set_settled_secs(&mut self, secs: u64) {
        self.settled_secs = secs;
    }

    /// True when the window may request a frame. False for a settled,
    /// visible window with nothing animating, and always false when the
    /// window is hidden/destroyed (tray-only).
    pub fn needs_frame(&self) -> bool {
        if !self.visible {
            return false;
        }
        if self.animating {
            return true;
        }
        self.settled_secs < SETTLE_SECS
    }
}

/// Check a report against the PERFORMANCE.md §1 hard caps.
pub fn check_budgets(
    scenario: ProbeScenario,
    avg_cpu_percent: f64,
    peak_rss_bytes: u64,
) -> Vec<String> {
    let mut violations = Vec::new();
    if avg_cpu_percent > scenario.cpu_cap_percent() {
        violations.push(format!(
            "cpu {avg_cpu_percent:.2}% over cap {:.1}% ({})",
            scenario.cpu_cap_percent(),
            scenario.as_str()
        ));
    }
    if peak_rss_bytes > scenario.mem_cap_bytes() {
        violations.push(format!(
            "rss {} over cap {} ({})",
            peak_rss_bytes,
            scenario.mem_cap_bytes(),
            scenario.as_str()
        ));
    }
    violations
}

fn read_own_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    if l.starts_with("VmRSS:") {
                        l.split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse::<u64>().ok())
                            .map(|kb| kb * 1024)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

fn read_own_cpu_times() -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        // (total jiffies, system uptime jiffies-ish via CLOCK_MONOTONIC).
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let end = stat.rfind(')')?;
        let fields: Vec<&str> = stat[end + 1..].split_whitespace().collect();
        // utime=field 14 (idx 12), stime=field 15 (idx 13) after comm.
        let utime: u64 = fields.get(12)?.parse().ok()?;
        let stime: u64 = fields.get(13)?.parse().ok()?;
        Some((utime + stime, cpu_clock_tick_hz()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn cpu_clock_tick_hz() -> u64 {
    100
}

fn sample_cpu_percent(prev: (u64, u64), now: (u64, u64), elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        return 0.0;
    }
    let dj = now.0.saturating_sub(prev.0) as f64;
    let hz = now.1.max(1) as f64;
    dj / hz / elapsed_secs * 100.0
}

/// Run the probe synchronously: settle, then sample own-process RSS/CPU
/// about 2×/s for `seconds`. `seconds == 0` samples once (for tests).
/// Frame counts come from the [`IdleFramePolicy`]: idle/tray request none.
pub fn run_probe(scenario: ProbeScenario, seconds: u64) -> ProbeReport {
    let policy = match scenario {
        ProbeScenario::Idle => IdleFramePolicy::idle_settled(),
        ProbeScenario::Tray => {
            let mut p = IdleFramePolicy::idle_settled();
            p.set_visible(false);
            p
        }
    };
    // A settled idle/tray window requests no frames; an animating window
    // would report policy.needs_frame() == true per sample.
    let frames_requested = if policy.needs_frame() {
        seconds.max(1) * 60
    } else {
        0
    };

    let mut rss_samples = Vec::new();
    let mut cpu_total = 0.0;
    let mut cpu_n = 0u32;
    let mut peak: u64 = 0;

    let mut prev_cpu = read_own_cpu_times();
    let mut prev_t = std::time::Instant::now();
    rss_samples.push(read_own_rss_bytes());

    let steps = (seconds * 2).max(1);
    let step_secs = if seconds == 0 { 0.0 } else { 1.0 / 2.0 };
    for _ in 0..steps {
        if step_secs > 0.0 {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let rss = read_own_rss_bytes();
        rss_samples.push(rss);
        peak = peak.max(rss);
        if let (Some(p), Some(n)) = (prev_cpu, read_own_cpu_times()) {
            let el = prev_t.elapsed().as_secs_f64();
            cpu_total += sample_cpu_percent(p, n, el);
            cpu_n += 1;
        }
        prev_cpu = read_own_cpu_times();
        prev_t = std::time::Instant::now();
        let _ = step_secs;
    }

    let avg_rss = if rss_samples.is_empty() {
        0
    } else {
        rss_samples.iter().sum::<u64>() / rss_samples.len() as u64
    };
    let avg_cpu = if cpu_n == 0 {
        0.0
    } else {
        cpu_total / cpu_n as f64
    };
    let violations = check_budgets(scenario, avg_cpu, peak);
    let fps = if seconds == 0 {
        0.0
    } else {
        frames_requested as f64 / seconds.max(1) as f64
    };
    ProbeReport {
        scenario,
        seconds,
        avg_cpu_percent: avg_cpu,
        avg_rss_bytes: avg_rss,
        peak_rss_bytes: peak,
        footprint_bytes: None,
        wakeups_per_sec: None,
        frames_requested,
        frames_per_second: fps,
        continuous_frames: frames_requested > 0,
        cpu_cap_percent: scenario.cpu_cap_percent(),
        mem_cap_bytes: scenario.mem_cap_bytes(),
        passed: violations.is_empty() && frames_requested == 0,
        violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_probe_args_parse() {
        let argv = vec![
            "--perf-probe".to_string(),
            "idle".to_string(),
            "--seconds".to_string(),
            "10".to_string(),
        ];
        assert_eq!(
            ProbeArgs::parse(&argv),
            Some(ProbeArgs {
                scenario: ProbeScenario::Idle,
                seconds: 10
            })
        );
        let argv = vec!["--perf-probe=tray".to_string(), "--seconds=5".to_string()];
        assert_eq!(
            ProbeArgs::parse(&argv),
            Some(ProbeArgs {
                scenario: ProbeScenario::Tray,
                seconds: 5
            })
        );
        let argv = vec!["--perf-probe".to_string(), "run5".to_string()];
        assert_eq!(ProbeArgs::parse(&argv), None);
        assert_eq!(ProbeArgs::parse(&[]), None);
    }

    #[test]
    fn perf_probe_idle_window_requests_no_frames_when_settled() {
        // Settled, nothing animating: no frames (the core M1.7 assertion).
        let mut p = IdleFramePolicy::idle_settled();
        assert!(!p.needs_frame());
        // Still settling: frames allowed.
        p.set_settled_secs(0);
        assert!(p.needs_frame());
        // Animating (streaming/avatar): frames allowed.
        p.set_settled_secs(SETTLE_SECS);
        p.set_animating(true);
        assert!(p.needs_frame());
        // Hidden/destroyed (tray-only): never requests frames.
        p.set_animating(false);
        p.set_visible(false);
        assert!(!p.needs_frame());
    }

    #[test]
    fn perf_probe_tray_never_requests_frames() {
        let mut p = IdleFramePolicy::idle_settled();
        p.set_visible(false);
        p.set_animating(true); // even "animating" content is gone: window destroyed
        assert!(!p.needs_frame());
    }

    #[test]
    fn perf_probe_json_format_has_documented_fields() {
        let r = run_probe(ProbeScenario::Idle, 0);
        assert_eq!(r.frames_requested, 0);
        assert!(!r.continuous_frames);
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).expect("valid JSON");
        for key in [
            "scenario",
            "seconds",
            "avg_cpu_percent",
            "avg_rss_bytes",
            "peak_rss_bytes",
            "footprint_bytes",
            "wakeups_per_sec",
            "frames_requested",
            "frames_per_second",
            "continuous_frames",
            "passed",
        ] {
            assert!(v.get(key).is_some(), "missing key {key}");
        }
        assert_eq!(v["scenario"], "idle");
        let r = run_probe(ProbeScenario::Tray, 0);
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).expect("valid JSON");
        assert_eq!(v["scenario"], "tray");
    }

    #[test]
    fn perf_probe_budgets_enforced() {
        // Under caps: passes.
        assert!(check_budgets(ProbeScenario::Idle, 0.1, 70 * 1024 * 1024).is_empty());
        assert!(check_budgets(ProbeScenario::Tray, 0.0, 30 * 1024 * 1024).is_empty());
        // Over caps: fails with a message naming the dimension.
        let v = check_budgets(ProbeScenario::Idle, 5.0, 70 * 1024 * 1024);
        assert!(v.iter().any(|s| s.contains("cpu")));
        let v = check_budgets(ProbeScenario::Tray, 0.0, 500 * 1024 * 1024);
        assert!(v.iter().any(|s| s.contains("rss")));
        // Probe report carries the caps from PERFORMANCE.md §1.
        let r = run_probe(ProbeScenario::Idle, 0);
        assert_eq!(r.cpu_cap_percent, IDLE_CPU_CAP_PERCENT);
        assert_eq!(r.mem_cap_bytes, IDLE_MEM_CAP_BYTES);
    }
}
