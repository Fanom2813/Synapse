//! M10 release acceptance (see docs/MILESTONES.md §M10).
//!
//! Offline integration through the real workspace crates for the M10
//! release flow: resumable onboarding into the first project, Markdown +
//! printable-HTML artifact/transcript export with secrets redacted, and
//! unsigned packaging/updater configuration without signing credentials.
//! No live keys, network, or GPU. Release-only perf probes stay in the
//! `m10_performance` target.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

use synapse_app::onboarding::{DetectionSnapshot, FirstLaunch, Step};
use synapse_app::updater::{
    check_for_update_with_endpoint, manifest_is_newer, update_config, UPDATE_MANIFEST_ENDPOINT,
};
use synapse_core::artifacts::{render_synthesis_report, ArtifactStore, ReportDraft};
use synapse_core::bus::{Envelope, Participant, Recipients};
use synapse_core::export::{
    export_artifact, export_transcript, printable_html, write_export_file, ExportFormat,
};
use synapse_core::redact::{fixtures, Redactor};

fn mem_store() -> synapse_store::Store {
    synapse_store::Store::open_in_memory().unwrap()
}

fn usable_detection() -> DetectionSnapshot {
    DetectionSnapshot {
        cli_agents: vec![synapse_app::onboarding::CliEntry {
            id: "claude".into(),
            label: "Claude Code".into(),
            logged_in: true,
        }],
        local_models: vec![],
        api_key_ok: false,
        done: true,
    }
}

fn redactor_with_fixtures() -> Redactor {
    let mut r = Redactor::new();
    r.add_secrets(fixtures::all());
    r
}

// ---------------------------------------------------------------------------
// 1. onboarding: progress, resume after restart, completion into project
// ---------------------------------------------------------------------------

#[test]
fn m10_onboarding_progress_resume_and_first_project() {
    let store = mem_store();
    assert!(FirstLaunch::should_show(&store), "fresh store shows wizard");

    // Drive partway, persisting as the UI does after every step.
    let mut first = FirstLaunch::open(&store);
    assert_eq!(first.flow().step(), Step::Hello);
    let (n, total) = first.flow().progress();
    assert_eq!((n, total), (1, 7));
    first.set_lead_name(&store, "Dottie");
    first.set_look(&store, "bean", "green", "diamond", "");
    first.press_key(&store, "Enter"); // Hello -> LeadName
    assert_eq!(first.flow().step(), Step::LeadName);
    first.advance(&store); // LeadName -> LeadLook
    assert_eq!(first.flow().step(), Step::LeadLook);

    // "Restart": a fresh handle resumes at the saved step with inputs.
    let mut resumed = FirstLaunch::open(&store);
    assert_eq!(resumed.flow().step(), Step::LeadLook);
    assert_eq!(resumed.flow().snapshot().lead_name, "Dottie");
    assert!(resumed.should_show_self(&store));
    assert!(resumed.flow().snapshot().step_index > 0);

    // Every step is skippable / keyboard-first and none wait on network:
    // Brains unblocks from a background detection snapshot, not a live probe.
    resumed.flow_mut().apply_detection(usable_detection());
    while resumed.flow().step() != Step::Brains {
        resumed.advance(&store);
    }
    assert!(resumed.can_continue(), "detection unblocks Brains offline");
    assert!(resumed.flow().blocked_hint().is_none());
    resumed.advance(&store); // Brains -> All set
    assert_eq!(resumed.flow().step(), Step::AllSet);
    assert!(resumed.can_continue());

    // Completion creates the first project, posts the lead introduction
    // into its room, and persists so returning users bypass onboarding.
    let done = resumed.complete(&store).expect("complete first project");
    assert!(!done.project_id.is_empty());
    assert!(!done.conversation_id.is_empty());
    assert!(done.introduction.contains("Dottie"));
    let project = store.get_project(&done.project_id).expect("project saved");
    assert_eq!(project.name, "First project");
    let messages = store
        .list_messages(&done.conversation_id, 10, 0)
        .expect("intro posted");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, done.introduction);
    assert!(!FirstLaunch::should_show(&store), "completed flow bypasses");
}

// ---------------------------------------------------------------------------
// 2. export: Markdown + printable HTML, secrets redacted
// ---------------------------------------------------------------------------

fn store_with_secret_report() -> (ArtifactStore, String) {
    let mut store = ArtifactStore::new();
    let draft = ReportDraft {
        summary: format!("Findings mention {} inline.", fixtures::OPENAI.1),
        findings: vec![("Costs".into(), "Fees rose `fast`.".into())],
        ..Default::default()
    };
    let body = render_synthesis_report("Market study", &draft);
    let a = store
        .create(
            "project_1",
            "Market study",
            "report",
            &body,
            "Initial synthesis.",
            Vec::new(),
            "run-1",
            1_700_000_000,
        )
        .unwrap();
    (store, a.id.clone())
}

fn secret_transcript() -> Vec<Envelope> {
    vec![
        Envelope::say(
            "m1",
            "conv",
            Participant::User,
            Recipients::Room,
            "What did you find?",
        ),
        Envelope::say(
            "m2",
            "conv",
            Participant::Agent("writer".into()),
            Recipients::Room,
            &format!("Token {} powered `growth`.", fixtures::GITHUB.1),
        ),
    ]
}

#[test]
fn m10_artifact_export_markdown_and_printable_html_redacted() {
    let (store, id) = store_with_secret_report();
    let redactor = redactor_with_fixtures();

    let md = export_artifact(&store, &id, ExportFormat::Markdown, &redactor).unwrap();
    assert!(md.contains("# Market study"), "got:\n{md}");
    assert!(md.ends_with('\n'));
    assert!(
        !md.contains(fixtures::OPENAI.1),
        "secret leaked in markdown"
    );
    assert!(md.contains("[redacted:"), "missing redaction marker");

    let html = export_artifact(&store, &id, ExportFormat::Html, &redactor).unwrap();
    assert!(html.contains("<title>Market study</title>"), "got:\n{html}");
    assert!(html.contains("@media print"), "HTML must be printable");
    assert!(!html.contains(fixtures::OPENAI.1), "secret leaked in HTML");
    assert!(!html.contains("<script>"), "bodies must be escaped");

    // Atomic file write lands under the project folder's exports dir.
    let dir = std::env::temp_dir().join(format!("synapse-m10-artifact-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = write_export_file(&dir, "market-study.md", &md).unwrap();
    assert_eq!(path, dir.join("exports").join("market-study.md"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), md);
    assert!(!path.with_extension("tmp").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn m10_transcript_export_markdown_and_printable_html_redacted() {
    let messages = secret_transcript();
    let redactor = redactor_with_fixtures();

    let md = export_transcript("Team talk", &messages, ExportFormat::Markdown, &redactor).unwrap();
    assert!(md.contains("# Team talk"), "got:\n{md}");
    assert!(md.contains("## writer"), "got:\n{md}");
    assert!(
        !md.contains(fixtures::GITHUB.1),
        "secret leaked in markdown"
    );

    let html = export_transcript("Team talk", &messages, ExportFormat::Html, &redactor).unwrap();
    assert!(html.contains("@media print"), "HTML must be printable");
    assert!(html.contains("<code>growth</code>"), "got:\n{html}");
    assert!(!html.contains(fixtures::GITHUB.1), "secret leaked in HTML");

    // Printable helper is a full document (OS print dialog -> PDF).
    let doc = printable_html("Talk", "<p>hi</p>");
    assert!(doc.contains("<!DOCTYPE html>"));
    assert!(doc.contains("@media print"));

    // Empty transcripts are a typed error, not an empty file.
    assert!(matches!(
        export_transcript("Empty", &[], ExportFormat::Markdown, &redactor),
        Err(synapse_core::export::ExportError::EmptyTranscript)
    ));
}

// ---------------------------------------------------------------------------
// 3. unsigned packaging + updater configuration (no signing credentials)
// ---------------------------------------------------------------------------

/// Serve each `(status, body)` once on 127.0.0.1 (offline loopback).
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

fn manifest(version: &str) -> String {
    let target = cargo_packager_updater::target().expect("test platform target");
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

#[test]
fn m10_unsigned_packaging_config_and_updater_without_credentials() {
    // Unsigned local builds carry no public key: nothing to sign or verify.
    let config = update_config();
    assert_eq!(config.endpoints.len(), 1);
    assert!(config.pubkey.is_empty(), "unsigned builds have no key");
    assert!(UPDATE_MANIFEST_ENDPOINT.starts_with("https://github.com/"));
    assert!(UPDATE_MANIFEST_ENDPOINT.ends_with("latest.json"));

    // Pure semver gate: newer manifests update, same/older do not.
    let target = cargo_packager_updater::target().expect("test platform target");
    let release: cargo_packager_updater::RemoteRelease =
        serde_json::from_str(&manifest("v9.9.9")).expect("manifest parses");
    assert!(target.contains('-'), "updater target looks platform-like");
    let older = "1.0.0".parse().expect("semver");
    let same = "9.9.9".parse().expect("semver");
    assert!(manifest_is_newer(&older, &release));
    assert!(!manifest_is_newer(&same, &release));

    // No manifest published yet (204): up to date, no credentials needed.
    let (url, handle) = serve_http(vec![(204, String::new())]);
    let found = check_for_update_with_endpoint("0.1.0", &url).expect("204 means no update");
    handle.join().expect("test server done");
    assert!(found.is_none());

    // A newer manifest is reported with its download URL + notes.
    let (url, handle) = serve_http(vec![(200, manifest("v9.9.9"))]);
    let found = check_for_update_with_endpoint("0.1.0", &url)
        .expect("updater check runs")
        .expect("newer release found");
    handle.join().expect("test server done");
    assert_eq!(found.version, "9.9.9");
    assert_eq!(
        found.download_url,
        "https://example.com/synapse-test.tar.gz"
    );
    assert_eq!(found.notes.as_deref(), Some("Test release"));

    // The checked-in packaging config builds unsigned bundles (no key
    // flags, per-platform formats documented, icons + resources present).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Packager.toml");
    let raw = std::fs::read_to_string(&root).expect("Packager.toml exists");
    assert!(raw.contains("before-packaging-command"));
    for section in ["[dmg]", "[nsis]", "[wix]", "[deb]", "[appimage]"] {
        assert!(raw.contains(section), "missing {section}");
    }
    assert!(
        !raw.contains("pubkey") && !raw.contains("private_key"),
        "unsigned default must not embed signing keys"
    );
    let workspace = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
    )
    .expect("workspace Cargo.toml exists");
    let version = workspace
        .lines()
        .find(|l| l.trim_start().starts_with("version"))
        .expect("workspace version");
    assert!(
        version.contains("0.1.0"),
        "packaging tracks workspace version"
    );
}
