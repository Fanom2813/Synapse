//! App updater: checks GitHub Releases for a newer signed bundle.
//!
//! The release workflow (`.github/workflows/release.yml`) publishes the
//! cargo-packager bundles plus a `latest.json` manifest to each GitHub
//! release. This module polls that manifest with
//! [`cargo_packager_updater`] and reports whether a newer version exists.
//! Download + install (signature verified against the release public key)
//! only runs when the user accepts an update in Settings.
//!
//! Unsigned local builds have no key and no manifest: [`check_for_update`]
//! then finds no endpoint / no newer release and returns `Ok(None)`.
//! The blocking HTTP check must run off the UI thread (e.g. a tokio
//! blocking thread via `synapse-app::runtime`); it never runs on a timer,
//! only when the user opens Settings → Updates or once per app start.

use std::time::Duration;

use cargo_packager_updater::{Config, RemoteRelease, Update, UpdaterBuilder};

/// Manifest published to every GitHub release by the release workflow.
pub const UPDATE_MANIFEST_ENDPOINT: &str =
    "https://github.com/Fanom2813/Synapse/releases/latest/download/latest.json";

/// Env var overriding the manifest URL (custom feeds, diagnostics).
pub const UPDATE_ENDPOINT_ENV: &str = "SYNAPSE_UPDATE_ENDPOINT";

/// Request timeout for the manifest check.
pub const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// A newer release found by [`check_for_update`].
#[derive(Debug, Clone)]
pub struct PendingUpdate {
    /// Installed version that was checked.
    pub current_version: String,
    /// New version available.
    pub version: String,
    /// Release notes, if the manifest has any.
    pub notes: Option<String>,
    /// Download URL for this platform.
    pub download_url: String,
}

impl From<Update> for PendingUpdate {
    fn from(update: Update) -> Self {
        Self {
            current_version: update.current_version.clone(),
            version: update.version.clone(),
            notes: update.body.clone(),
            download_url: update.download_url.to_string(),
        }
    }
}

/// Manifest endpoint in effect: `${SYNAPSE_UPDATE_ENDPOINT}` or the default.
pub fn update_endpoint() -> String {
    std::env::var(UPDATE_ENDPOINT_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| UPDATE_MANIFEST_ENDPOINT.to_string())
}

/// Updater config for the current app. The public key is empty for unsigned
/// local builds; signature verification only matters on download, and there
/// is nothing newer to download without a published manifest.
pub fn update_config() -> Config {
    Config {
        endpoints: vec![update_endpoint()
            .parse()
            .expect("update endpoint must be a valid URL")],
        pubkey: String::new(),
        ..Default::default()
    }
}

/// `true` when the manifest advertises a version newer than `current`.
/// Pure (no I/O): unit-testable without network.
pub fn manifest_is_newer(
    current: &cargo_packager_updater::semver::Version,
    release: &RemoteRelease,
) -> bool {
    release.version > *current
}

/// Check the manifest for a newer release. Returns `Ok(None)` when up to
/// date or when the endpoint answers 204 (no update published yet).
/// Blocking: call off the UI thread.
pub fn check_for_update(
    current_version: &str,
) -> Result<Option<PendingUpdate>, cargo_packager_updater::Error> {
    check_for_update_with_endpoint(current_version, &update_endpoint())
}

/// [`check_for_update`] against an explicit manifest URL (custom feeds, tests).
/// Blocking: call off the UI thread.
pub fn check_for_update_with_endpoint(
    current_version: &str,
    endpoint: &str,
) -> Result<Option<PendingUpdate>, cargo_packager_updater::Error> {
    let current = current_version
        .trim_start_matches(['v', 'V'])
        .parse()
        .map_err(cargo_packager_updater::Error::Semver)?;
    let config = Config {
        endpoints: vec![endpoint.parse()?],
        pubkey: String::new(),
        ..Default::default()
    };
    let found = UpdaterBuilder::new(current, config)
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()?
        .check()?;
    Ok(found.map(PendingUpdate::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    /// Serve each `(status, body)` once, in order, on 127.0.0.1.
    /// Returns the manifest URL plus a handle to join after the check.
    fn serve_http(responses: Vec<(u16, String)>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("test server addr");
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept test request");
                let mut buf = vec![0u8; 8192];
                let _ = stream.read(&mut buf);
                let reason = if status == 200 { "OK" } else { "No Content" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{addr}/latest.json"), handle)
    }

    fn manifest(version: &str, target: &str) -> String {
        format!(
            concat!(
                r#"{{"version":"{version}","notes":"Test release","#,
                r#""pub_date":"2026-10-14T00:00:00Z","#,
                r#""platforms":{{"{target}":{{"url":"#,
                r#""https://example.com/synapse-test.tar.gz","#,
                r#""signature":"sig","format":"appimage"}}}}}}"#,
            ),
            version = version,
            target = target
        )
    }

    fn current_target() -> String {
        cargo_packager_updater::target().expect("test platform has an updater target")
    }

    #[test]
    fn updater_finds_newer_release() {
        let target = current_target();
        let (url, handle) = serve_http(vec![(200, manifest("v9.9.9", &target))]);
        let found = check_for_update_with_endpoint("0.1.0", &url)
            .expect("updater check runs")
            .expect("newer release found");
        handle.join().expect("test server done");
        assert_eq!(found.version, "9.9.9");
        assert_eq!(found.current_version, "0.1.0");
        assert_eq!(
            found.download_url,
            "https://example.com/synapse-test.tar.gz"
        );
        assert_eq!(found.notes.as_deref(), Some("Test release"));
    }

    #[test]
    fn updater_returns_none_when_up_to_date() {
        let target = current_target();
        let (url, handle) = serve_http(vec![(200, manifest("0.1.0", &target))]);
        let found = check_for_update_with_endpoint("0.1.0", &url).expect("updater check runs");
        handle.join().expect("test server done");
        assert!(found.is_none(), "same version is not an update");
    }

    #[test]
    fn updater_returns_none_for_older_manifest() {
        let target = current_target();
        let (url, handle) = serve_http(vec![(200, manifest("0.0.1", &target))]);
        let found = check_for_update_with_endpoint("0.1.0", &url).expect("updater check runs");
        handle.join().expect("test server done");
        assert!(found.is_none(), "older manifest is not an update");
    }

    #[test]
    fn updater_returns_none_on_204_no_content() {
        let (url, handle) = serve_http(vec![(204, String::new())]);
        let found = check_for_update_with_endpoint("0.1.0", &url).expect("204 means no update");
        handle.join().expect("test server done");
        assert!(found.is_none());
    }

    #[test]
    fn updater_rejects_invalid_version() {
        let result = check_for_update_with_endpoint("not-a-version", "http://127.0.0.1:1/x.json");
        assert!(
            matches!(result, Err(cargo_packager_updater::Error::Semver(_))),
            "garbage version is a Semver error, got: {result:?}"
        );
    }

    #[test]
    fn updater_manifest_is_newer_compares_semver() {
        let target = current_target();
        let release: RemoteRelease =
            serde_json::from_str(&manifest("v1.2.3", &target)).expect("static manifest parses");
        let older = "1.2.2".parse().expect("semver");
        let same = "1.2.3".parse().expect("semver");
        assert!(manifest_is_newer(&older, &release));
        assert!(!manifest_is_newer(&same, &release));
    }

    #[test]
    fn updater_endpoint_defaults_to_github_releases() {
        assert!(UPDATE_MANIFEST_ENDPOINT.starts_with("https://github.com/"));
        assert!(UPDATE_MANIFEST_ENDPOINT.ends_with("latest.json"));
        assert_eq!(update_config().endpoints.len(), 1);
    }
}
