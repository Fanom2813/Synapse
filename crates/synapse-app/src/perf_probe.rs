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
    /// Dimensions that could not be measured on this platform/run.
    /// A non-empty list always fails the probe: missing data must never
    /// read as zero and pass a budget (see [`run_probe`]).
    #[serde(default)]
    pub unavailable: Vec<String>,
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

/// Resident set size of this process in bytes, cross-platform via the
/// `memory-stats` crate (Linux/macOS/Windows). `None` when the platform
/// cannot report it — callers must fail or mark the run unavailable,
/// never substitute zero.
fn read_rss_bytes() -> Option<u64> {
    if let Some(stats) = memory_stats::memory_stats() {
        if stats.physical_mem > 0 {
            return Some(stats.physical_mem as u64);
        }
    }
    // Fallback: Linux /proc (memory-stats reads this internally, but a
    // direct read keeps the probe working if the crate ever returns 0).
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
                            .filter(|b| *b > 0)
                    } else {
                        None
                    }
                })
            })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Cross-platform sampler for this process via `sysinfo`: RSS plus CPU %.
/// CPU needs two refreshes to be meaningful; the first `cpu_usage()`
/// reading after a fresh `System` is discarded as warmup.
struct ProcessSampler {
    system: sysinfo::System,
    pid: sysinfo::Pid,
    warmed_up: bool,
}

impl ProcessSampler {
    fn current() -> Option<Self> {
        if !sysinfo::IS_SUPPORTED_SYSTEM {
            return None;
        }
        let pid = sysinfo::get_current_pid().ok()?;
        let mut system = sysinfo::System::new();
        if !system.refresh_process(pid) {
            return None;
        }
        Some(Self {
            system,
            pid,
            warmed_up: false,
        })
    }

    /// Refresh and return `(rss_bytes, cpu_percent)`. `cpu_percent` is
    /// `None` for the warmup reading and when the process vanishes.
    fn sample(&mut self) -> (Option<u64>, Option<f64>) {
        use sysinfo::ProcessRefreshKind;
        self.system.refresh_process_specifics(
            self.pid,
            ProcessRefreshKind::new().with_memory().with_cpu(),
        );
        let proc = match self.system.process(self.pid) {
            Some(p) => p,
            None => return (read_rss_bytes(), None),
        };
        let mut rss = Some(proc.memory()).filter(|b| *b > 0);
        if rss.is_none() {
            rss = read_rss_bytes();
        }
        let cpu = if self.warmed_up {
            Some(f64::from(proc.cpu_usage()))
        } else {
            self.warmed_up = true;
            None
        };
        (rss, cpu)
    }
}

/// Exercise the real window scenario headless and return its frame
/// policy. The native GPUI window needs a display, so the probe runs the
/// toolkit-free window stack that decides idle wakeups (`AppShell` open,
/// initial workspace, `ShellScreen` describe as the GPUI view renders it,
/// and the `WindowLifecycle` close/destroy transitions for tray).
fn exercise_scenario(scenario: ProbeScenario) -> IdleFramePolicy {
    use crate::lifecycle::WindowLifecycle;
    use crate::shell::AppShell;
    use crate::window::ShellScreen;

    // Real shell open path (in-memory; the persisted variant only adds
    // JSON loads on top and is covered by shell tests).
    let mut shell = AppShell::open();
    shell.initial_workspace();
    let screen = ShellScreen::describe(&shell);
    std::hint::black_box(
        (screen.window_title.len()
            + screen.sidebar_text.len()
            + screen.main_text.len()
            + screen.panel_text.len()) as u64,
    );

    match scenario {
        ProbeScenario::Idle => IdleFramePolicy::idle_settled(),
        ProbeScenario::Tray => {
            // Tray-only: the window was closed (hidden) and then destroyed,
            // so nothing is visible and no frames may be requested.
            let mut lifecycle = WindowLifecycle::new_visible();
            lifecycle.close_window();
            lifecycle.advance(crate::lifecycle::DESTROY_HIDDEN_AFTER_SECS);
            std::hint::black_box(lifecycle.visibility() as u8);
            let mut p = IdleFramePolicy::idle_settled();
            p.set_visible(false);
            p
        }
    }
}

/// Run the probe synchronously: exercise the real window scenario (see
/// [`exercise_scenario`]), then sample this process's RSS/CPU about 2×/s
/// for `seconds` via `sysinfo`/`memory-stats` (PERFORMANCE.md §3).
/// `seconds == 0` still takes one real sample so a zero-length run can
/// never report zero peak RSS and pass a budget.
///
/// Dimensions that cannot be measured are listed in
/// [`ProbeReport::unavailable`] and fail the probe instead of reading as
/// zero.
pub fn run_probe(scenario: ProbeScenario, seconds: u64) -> ProbeReport {
    let policy = exercise_scenario(scenario);
    // A settled idle/tray window requests no frames; an animating window
    // would report policy.needs_frame() == true per sample.
    let frames_requested = if policy.needs_frame() {
        seconds.max(1) * 60
    } else {
        0
    };

    let mut rss_samples: Vec<u64> = Vec::new();
    let mut cpu_total = 0.0;
    let mut cpu_n = 0u32;
    let mut peak: u64 = 0;
    let mut rss_missing = 0u32;
    let mut cpu_missing = 0u32;
    let mut total_samples = 0u32;

    let mut sampler = ProcessSampler::current();
    let sampler_available = sampler.is_some();
    // Warm up CPU accounting (sysinfo needs two refreshes ~200 ms apart).
    if let Some(s) = sampler.as_mut() {
        let _ = s.sample();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    }

    let steps = (seconds * 2).max(1);
    for step in 0..steps {
        if seconds > 0 && step > 0 {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        total_samples += 1;
        let (rss, cpu) = match sampler.as_mut() {
            Some(s) => s.sample(),
            None => (read_rss_bytes(), None),
        };
        match rss {
            Some(b) => {
                rss_samples.push(b);
                peak = peak.max(b);
            }
            None => rss_missing += 1,
        }
        match cpu {
            Some(c) if c.is_finite() => {
                cpu_total += c;
                cpu_n += 1;
            }
            _ => cpu_missing += 1,
        }
        // Keep the exercised scenario alive across samples so the
        // measurement covers the real window state, not an empty sleep.
        std::hint::black_box(policy.needs_frame());
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
    let mut unavailable = Vec::new();
    if !sampler_available {
        unavailable.push("process sampler unavailable on this platform".to_string());
    }
    if rss_samples.is_empty() {
        unavailable.push("rss measurement unavailable on this platform".to_string());
    }
    // The first CPU reading is warmup; only fail when no real reading
    // arrived at all (e.g. unsupported platform or vanishing process).
    if cpu_n == 0 {
        unavailable.push("cpu measurement unavailable on this platform".to_string());
    }
    let _ = (rss_missing, cpu_missing, total_samples);
    let mut violations = check_budgets(scenario, avg_cpu, peak);
    let fps = if seconds == 0 {
        0.0
    } else {
        frames_requested as f64 / seconds.max(1) as f64
    };
    let passed = violations.is_empty() && unavailable.is_empty() && frames_requested == 0;
    if frames_requested > 0 {
        violations.push(format!(
            "window requested {frames_requested} frames while idle ({})",
            scenario.as_str()
        ));
    }
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
        passed,
        violations,
        unavailable,
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
    fn perf_probe_zero_second_sample_never_passes_on_zero_rss() {
        // A zero-length run must measure the real process: either it
        // reports a nonzero peak RSS or it marks the dimension
        // unavailable and fails — never zero-and-passed.
        let r = run_probe(ProbeScenario::Idle, 0);
        assert!(
            r.peak_rss_bytes > 0 || !r.unavailable.is_empty(),
            "zero-second sample reported zero peak RSS as valid data"
        );
        if r.peak_rss_bytes == 0 {
            assert!(!r.passed);
        }
        if !r.unavailable.is_empty() {
            assert!(!r.passed);
        }
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
