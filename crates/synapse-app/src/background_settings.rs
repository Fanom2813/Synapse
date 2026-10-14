//! Background + launch settings (M9 task 5, UI.md §2.5, ARCHITECTURE.md §5).
//!
//! Toolkit-free state so it runs headless in tests. The GPUI shell wires it
//! up: Settings → Background shows the three toggles, `auto-launch` applies
//! launch-at-login on the OS, and the window shell consults
//! [`BackgroundSettings::keep_running_on_close`] before quitting.

/// Store keys (one source of truth lives here, like `quick_ask.rs`).
pub const KEEP_RUNNING_ON_CLOSE_KEY: &str = "background_keep_running_on_close";
pub const LAUNCH_AT_LOGIN_KEY: &str = "background_launch_at_login";
pub const MENU_BAR_ONLY_KEY: &str = "background_menu_bar_only";

/// App + helper name registered with the OS for launch at login.
pub const AUTO_LAUNCH_APP_NAME: &str = "Synapse";

/// What the close button does when "keep running" is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseEffect {
    /// Hide to the tray/menu bar and keep runs going.
    HideToTray,
    /// Quit the app.
    Quit,
}

/// Which background capabilities the current OS supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformSupport {
    pub launch_at_login: bool,
    pub menu_bar_only: bool,
}

impl PlatformSupport {
    pub fn for_os(os: &str) -> Self {
        match os {
            "macos" => Self {
                launch_at_login: true,
                menu_bar_only: true,
            },
            "windows" | "linux" => Self {
                launch_at_login: true,
                menu_bar_only: false,
            },
            _ => Self {
                launch_at_login: false,
                menu_bar_only: false,
            },
        }
    }

    pub fn current() -> Self {
        Self::for_os(std::env::consts::OS)
    }

    /// Human explanation for an unsupported option (shown under the row).
    pub fn menu_bar_only_explanation(os: &str) -> &'static str {
        match os {
            "macos" => "Hides the dock icon and keeps Synapse in the menu bar only.",
            "windows" | "linux" => {
                "Menu-bar-only mode is supported on macOS only. On this system Synapse keeps a tray icon while running in the background."
            }
            _ => "Menu-bar-only mode is not supported on this platform.",
        }
    }

    pub fn launch_at_login_explanation(os: &str) -> &'static str {
        match os {
            "macos" => "Starts Synapse when you log in (Launch Agent).",
            "windows" => "Starts Synapse when you sign in (registry, current user).",
            "linux" => "Starts Synapse when you log in (XDG autostart entry).",
            _ => "Launch at login is not supported on this platform.",
        }
    }
}

/// Persisted background prefs. Defaults: keep running on close (tray-first
/// app), no auto-launch, no menu-bar-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundSettings {
    pub keep_running_on_close: bool,
    pub launch_at_login: bool,
    pub menu_bar_only: bool,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            keep_running_on_close: true,
            launch_at_login: false,
            menu_bar_only: false,
        }
    }
}

impl BackgroundSettings {
    pub fn load(store: &synapse_store::Store) -> Self {
        let mut out = Self::default();
        if let Ok(Some(raw)) = store.setting_get(KEEP_RUNNING_ON_CLOSE_KEY) {
            let v: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            out.keep_running_on_close = v.as_bool().unwrap_or_else(|| raw.trim() == "true");
        }
        if let Ok(Some(raw)) = store.setting_get(LAUNCH_AT_LOGIN_KEY) {
            let v: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            out.launch_at_login = v.as_bool().unwrap_or_else(|| raw.trim() == "true");
        }
        if let Ok(Some(raw)) = store.setting_get(MENU_BAR_ONLY_KEY) {
            let v: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            out.menu_bar_only = v.as_bool().unwrap_or_else(|| raw.trim() == "true");
        }
        // Menu-bar-only only makes sense on macOS; never load it as "on"
        // elsewhere (stale pref from a synced profile).
        if out.menu_bar_only && !PlatformSupport::current().menu_bar_only {
            out.menu_bar_only = false;
        }
        out
    }

    pub fn save(&self, store: &synapse_store::Store) -> Result<(), String> {
        // Persist as set; `load` (and the section view-model) resolve what is
        // effective on this OS, so a pref synced from a Mac is not lost.
        let to_save = self.clone();
        for (key, value) in [
            (KEEP_RUNNING_ON_CLOSE_KEY, to_save.keep_running_on_close),
            (LAUNCH_AT_LOGIN_KEY, to_save.launch_at_login),
            (MENU_BAR_ONLY_KEY, to_save.menu_bar_only),
        ] {
            store
                .setting_set(key, &serde_json::to_string(&value).unwrap_or_default())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// What the close button does under these prefs.
    pub fn close_effect(&self) -> CloseEffect {
        if self.keep_running_on_close {
            CloseEffect::HideToTray
        } else {
            CloseEffect::Quit
        }
    }

    /// Effective menu-bar-only state: desired AND supported on this OS.
    pub fn effective_menu_bar_only(&self) -> bool {
        self.effective_menu_bar_only_on(std::env::consts::OS)
    }

    pub fn effective_menu_bar_only_on(&self, os: &str) -> bool {
        self.menu_bar_only && PlatformSupport::for_os(os).menu_bar_only
    }
}

/// OS launch-at-login backend (real `auto-launch` in the shell, memory in
/// tests) so Settings can reflect the *effective* platform state.
pub trait AutoLaunchBackend {
    fn is_enabled(&self) -> Result<bool, String>;
    fn enable(&mut self) -> Result<(), String>;
    fn disable(&mut self) -> Result<(), String>;
}

/// In-memory backend for tests and headless use.
#[derive(Debug, Clone, Default)]
pub struct MemoryAutoLaunch {
    enabled: bool,
    pub fail_next: Option<String>,
}

impl MemoryAutoLaunch {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            fail_next: None,
        }
    }

    pub fn fail_next(mut self, message: &str) -> Self {
        self.fail_next = Some(message.to_string());
        self
    }
}

impl AutoLaunchBackend for MemoryAutoLaunch {
    fn is_enabled(&self) -> Result<bool, String> {
        if let Some(msg) = &self.fail_next {
            return Err(msg.clone());
        }
        Ok(self.enabled)
    }

    fn enable(&mut self) -> Result<(), String> {
        if let Some(msg) = self.fail_next.take() {
            return Err(msg);
        }
        self.enabled = true;
        Ok(())
    }

    fn disable(&mut self) -> Result<(), String> {
        if let Some(msg) = self.fail_next.take() {
            return Err(msg);
        }
        self.enabled = false;
        Ok(())
    }
}

/// Live backend over the `auto-launch` crate (LaunchAgent / registry / XDG).
pub struct SystemAutoLaunch {
    inner: auto_launch::AutoLaunch,
}

impl SystemAutoLaunch {
    pub fn new() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let inner = auto_launch::AutoLaunchBuilder::new()
            .set_app_name(AUTO_LAUNCH_APP_NAME)
            .set_app_path(exe.to_string_lossy().as_ref())
            .set_args(&["--minimized"])
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { inner })
    }
}

impl AutoLaunchBackend for SystemAutoLaunch {
    fn is_enabled(&self) -> Result<bool, String> {
        self.inner.is_enabled().map_err(|e| e.to_string())
    }

    fn enable(&mut self) -> Result<(), String> {
        self.inner.enable().map_err(|e| e.to_string())
    }

    fn disable(&mut self) -> Result<(), String> {
        self.inner.disable().map_err(|e| e.to_string())
    }
}

/// Launch-at-login as the user sees it: what they asked for vs what the OS
/// reports. `effective` is `None` when the OS state could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchState {
    pub desired: bool,
    pub effective: Option<bool>,
    pub error: Option<String>,
}

impl LaunchState {
    pub fn probe(settings: &BackgroundSettings, backend: &dyn AutoLaunchBackend) -> Self {
        match backend.is_enabled() {
            Ok(effective) => Self {
                desired: settings.launch_at_login,
                effective: Some(effective),
                error: None,
            },
            Err(message) => Self {
                desired: settings.launch_at_login,
                effective: None,
                error: Some(message),
            },
        }
    }

    /// True when the stored pref disagrees with the OS (user changed it
    /// outside Synapse, or the last apply failed).
    pub fn is_mismatched(&self) -> bool {
        matches!(self.effective, Some(effective) if effective != self.desired)
    }

    pub fn mismatch_note(&self) -> Option<String> {
        if !self.is_mismatched() {
            return None;
        }
        let effective = self.effective.unwrap_or(false);
        if effective {
            Some(
                "Launch at login is on in the OS but off in Synapse settings. Turn the toggle on to keep them in sync, or turn it off in your system settings."
                    .to_string(),
            )
        } else {
            Some(
                "Synapse is set to launch at login, but the OS reports it is off (it may have been removed outside Synapse). Toggle it off and on again to repair."
                    .to_string(),
            )
        }
    }
}

/// Apply the stored launch-at-login pref to the OS. Returns the error
/// message when the OS call fails (settings stay as the user set them so a
/// retry shows the mismatch note).
pub fn apply_launch_at_login(
    settings: &BackgroundSettings,
    backend: &mut dyn AutoLaunchBackend,
) -> Result<(), String> {
    if settings.launch_at_login {
        backend.enable()
    } else {
        backend.disable()
    }
}

/// One row in Settings → Background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundRow {
    pub title: &'static str,
    pub description: String,
    /// Toggle position as shown (for launch-at-login: the OS state when
    /// readable, else the stored pref).
    pub toggled_on: bool,
    /// False when the option is unsupported here (row is disabled).
    pub interactive: bool,
    /// Footnote shown under the row (unsupported reason, mismatch, error).
    pub footnote: Option<String>,
}

/// View-model for Settings → Background: three toggles that reflect the
/// effective platform state and explain unsupported options.
#[derive(Debug, Clone)]
pub struct BackgroundSettingsSection {
    pub settings: BackgroundSettings,
    pub launch: LaunchState,
    pub os: &'static str,
}

impl BackgroundSettingsSection {
    pub fn new(settings: BackgroundSettings, launch: LaunchState, os: &'static str) -> Self {
        Self {
            settings,
            launch,
            os,
        }
    }

    /// Load prefs + probe the OS backend in one step (shell entry point).
    pub fn load(
        store: &synapse_store::Store,
        backend: &dyn AutoLaunchBackend,
        os: &'static str,
    ) -> Self {
        let settings = BackgroundSettings::load(store);
        let launch = LaunchState::probe(&settings, backend);
        Self::new(settings, launch, os)
    }

    pub fn rows(&self) -> Vec<BackgroundRow> {
        let support = PlatformSupport::for_os(self.os);
        let mut rows = vec![BackgroundRow {
            title: "Keep running on close",
            description: "Closing the window hides Synapse to the tray and keeps runs going. Turn off to quit when the window closes."
                .to_string(),
            toggled_on: self.settings.keep_running_on_close,
            interactive: true,
            footnote: None,
        }];
        // Launch at login: show the OS state when readable.
        let toggled_on = self.launch.effective.unwrap_or(self.launch.desired);
        let mut footnote: Option<String> = None;
        if !support.launch_at_login {
            footnote = Some(PlatformSupport::launch_at_login_explanation(self.os).to_string());
        } else if let Some(note) = self.launch.mismatch_note() {
            footnote = Some(note);
        } else if let Some(err) = &self.launch.error {
            footnote = Some(format!("Could not read the system setting: {err}"));
        }
        rows.push(BackgroundRow {
            title: "Launch at login",
            description: PlatformSupport::launch_at_login_explanation(self.os).to_string(),
            toggled_on,
            interactive: support.launch_at_login,
            footnote,
        });
        // Menu-bar-only: macOS only; elsewhere disabled with an explanation.
        let (toggled_on, interactive, footnote) = if support.menu_bar_only {
            (
                self.settings.menu_bar_only,
                true,
                Some(PlatformSupport::menu_bar_only_explanation(self.os).to_string()),
            )
        } else {
            (
                false,
                false,
                Some(PlatformSupport::menu_bar_only_explanation(self.os).to_string()),
            )
        };
        rows.push(BackgroundRow {
            title: "Menu bar only",
            description: "Hide the dock icon and run from the menu bar.".to_string(),
            toggled_on,
            interactive,
            footnote,
        });
        rows
    }

    /// Banner-level notices (mismatch / unreadable OS state).
    pub fn notices(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(note) = self.launch.mismatch_note() {
            out.push(note);
        } else if let Some(err) = &self.launch.error {
            out.push(format!("Launch-at-login status is unknown: {err}"));
        }
        out
    }

    /// Toggle "keep running on close" and persist. Always succeeds (local).
    pub fn set_keep_running(
        &mut self,
        store: &synapse_store::Store,
        on: bool,
    ) -> Result<(), String> {
        self.settings.keep_running_on_close = on;
        self.settings.save(store)
    }

    /// Toggle launch at login: persist the pref, apply to the OS, and
    /// re-probe so the row shows the effective state.
    pub fn set_launch_at_login(
        &mut self,
        store: &synapse_store::Store,
        backend: &mut dyn AutoLaunchBackend,
        on: bool,
    ) -> Result<(), String> {
        self.settings.launch_at_login = on;
        self.settings.save(store)?;
        if let Err(message) = apply_launch_at_login(&self.settings, backend) {
            self.launch = LaunchState::probe(&self.settings, backend);
            return Err(message);
        }
        self.launch = LaunchState::probe(&self.settings, backend);
        Ok(())
    }

    /// Toggle menu-bar-only (macOS only). No-op error elsewhere.
    pub fn set_menu_bar_only(
        &mut self,
        store: &synapse_store::Store,
        on: bool,
    ) -> Result<(), String> {
        if on && !PlatformSupport::for_os(self.os).menu_bar_only {
            return Err(PlatformSupport::menu_bar_only_explanation(self.os).to_string());
        }
        self.settings.menu_bar_only = on;
        self.settings.save(store)
    }
}

#[cfg(test)]
mod background_settings_tests {
    use super::*;

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().unwrap()
    }

    #[test]
    fn background_settings_defaults_match_docs() {
        let s = BackgroundSettings::default();
        assert!(s.keep_running_on_close);
        assert!(!s.launch_at_login);
        assert!(!s.menu_bar_only);
        assert_eq!(s.close_effect(), CloseEffect::HideToTray);
        let off = BackgroundSettings {
            keep_running_on_close: false,
            ..s
        };
        assert_eq!(off.close_effect(), CloseEffect::Quit);
    }

    #[test]
    fn background_settings_roundtrip_persists() {
        let store = mem_store();
        let loaded = BackgroundSettings::load(&store);
        assert_eq!(loaded, BackgroundSettings::default());
        let mut s = loaded;
        s.keep_running_on_close = false;
        s.launch_at_login = true;
        s.save(&store).unwrap();
        let again = BackgroundSettings::load(&store);
        assert!(!again.keep_running_on_close);
        assert!(again.launch_at_login);
        // Toggling back persists too.
        let mut back = again;
        back.keep_running_on_close = true;
        back.launch_at_login = false;
        back.save(&store).unwrap();
        assert_eq!(
            BackgroundSettings::load(&store),
            BackgroundSettings::default()
        );
    }

    #[test]
    fn background_settings_launch_reflects_effective_platform_state() {
        let store = mem_store();
        // User wants launch at login, but the OS reports off → mismatch note.
        let settings = BackgroundSettings {
            launch_at_login: true,
            ..BackgroundSettings::default()
        };
        let backend = MemoryAutoLaunch::new(false);
        let state = LaunchState::probe(&settings, &backend);
        assert!(state.is_mismatched());
        let note = state.mismatch_note().expect("mismatch explained");
        assert!(note.contains("OS reports it is off"), "{note}");

        // Reverse mismatch: OS on, pref off.
        let settings_off = BackgroundSettings::default();
        let backend_on = MemoryAutoLaunch::new(true);
        let state = LaunchState::probe(&settings_off, &backend_on);
        assert!(state.is_mismatched());
        assert!(state.mismatch_note().unwrap().contains("on in the OS"));

        // In sync: no note, row shows the effective state.
        let section = BackgroundSettingsSection::new(
            BackgroundSettings::default(),
            LaunchState::probe(
                &BackgroundSettings::default(),
                &MemoryAutoLaunch::new(false),
            ),
            "linux",
        );
        assert!(section.notices().is_empty());
        let rows = section.rows();
        let launch = rows.iter().find(|r| r.title == "Launch at login").unwrap();
        assert!(!launch.toggled_on);
        assert!(launch.interactive);

        // Applying the pref drives the backend; section re-probes.
        let mut section = BackgroundSettingsSection::new(
            BackgroundSettings::default(),
            LaunchState::probe(
                &BackgroundSettings::default(),
                &MemoryAutoLaunch::new(false),
            ),
            "linux",
        );
        let mut backend = MemoryAutoLaunch::new(false);
        section
            .set_launch_at_login(&store, &mut backend, true)
            .unwrap();
        assert!(backend.is_enabled().unwrap());
        assert!(BackgroundSettings::load(&store).launch_at_login);
        assert!(!section.launch.is_mismatched());

        // OS failure surfaces as an error + footnote, pref kept for retry.
        let mut section = BackgroundSettingsSection::new(
            BackgroundSettings::default(),
            LaunchState::probe(
                &BackgroundSettings::default(),
                &MemoryAutoLaunch::new(false),
            ),
            "linux",
        );
        let mut failing = MemoryAutoLaunch::new(false).fail_next("registry denied");
        let err = section
            .set_launch_at_login(&store, &mut failing, true)
            .unwrap_err();
        assert!(err.contains("registry denied"), "{err}");
    }

    #[test]
    fn background_settings_menu_bar_only_supported_on_macos() {
        let store = mem_store();
        let mut section = BackgroundSettingsSection::new(
            BackgroundSettings::default(),
            LaunchState {
                desired: false,
                effective: Some(false),
                error: None,
            },
            "macos",
        );
        let rows = section.rows();
        let row = rows.iter().find(|r| r.title == "Menu bar only").unwrap();
        assert!(row.interactive);
        assert!(!row.toggled_on);
        section.set_menu_bar_only(&store, true).unwrap();
        assert!(section.settings.effective_menu_bar_only_on("macos"));
        let rows = section.rows();
        let row = rows.iter().find(|r| r.title == "Menu bar only").unwrap();
        assert!(row.toggled_on);
    }

    #[test]
    fn background_settings_menu_bar_only_unsupported_off_macos() {
        for os in ["windows", "linux", "freebsd"] {
            let section = BackgroundSettingsSection::new(
                BackgroundSettings::default(),
                LaunchState {
                    desired: false,
                    effective: Some(false),
                    error: None,
                },
                os,
            );
            let rows = section.rows();
            let row = rows.iter().find(|r| r.title == "Menu bar only").unwrap();
            assert!(!row.interactive, "{os} should disable the row");
            assert!(!row.toggled_on, "{os} shows off");
            let footnote = row.footnote.as_deref().unwrap_or("");
            assert!(
                footnote.contains("macOS only") || footnote.contains("not supported"),
                "{os} explains why: {footnote}"
            );
        }
        // Enabling off-macOS is rejected with the same explanation.
        let store = mem_store();
        let mut section = BackgroundSettingsSection::new(
            BackgroundSettings::default(),
            LaunchState {
                desired: false,
                effective: Some(false),
                error: None,
            },
            "linux",
        );
        let err = section.set_menu_bar_only(&store, true).unwrap_err();
        assert!(err.contains("macOS only"), "{err}");
        // Keep-running toggle still works everywhere.
        section.set_keep_running(&store, false).unwrap();
        assert_eq!(section.settings.close_effect(), CloseEffect::Quit);
        assert!(!BackgroundSettings::load(&store).keep_running_on_close);
    }

    #[test]
    fn background_settings_section_load_probes_os_state() {
        let store = mem_store();
        BackgroundSettings {
            launch_at_login: true,
            ..BackgroundSettings::default()
        }
        .save(&store)
        .unwrap();
        // Backend disagrees → section surfaces the mismatch notice.
        let backend = MemoryAutoLaunch::new(false);
        let section = BackgroundSettingsSection::load(&store, &backend, "windows");
        assert!(section.launch.is_mismatched());
        assert_eq!(section.notices().len(), 1);
        let rows = section.rows();
        assert_eq!(rows.len(), 3);
        for title in ["Keep running on close", "Launch at login", "Menu bar only"] {
            assert!(rows.iter().any(|r| r.title == title), "missing {title}");
        }
    }
}
