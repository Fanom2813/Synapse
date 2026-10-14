//! Shared secret redactor (see docs/ARCHITECTURE.md §6).
//!
//! Runs on everything that leaves the app for a model or gets stored:
//! file contents, shell output, MCP/CLI results, fetched pages, tool-call
//! args/results, logs, the audit log, error messages, memory writes, exports.
//!
//! Layer 1: known secret values (exact match, Aho-Corasick) → `[redacted:<NAME>]`.
//! Layer 2: built-in secret patterns (gitleaks-style rules, `regex`) → `[redacted:<rule-id>]`.
//! Layer 3: default deny paths ([`is_denied_path`]) are never readable by agents.

use std::path::Path;

use aho_corasick::AhoCorasick;
use regex::Regex;

/// One redaction pattern: rule id + compiled regex.
#[derive(Debug)]
struct Pattern {
    rule_id: &'static str,
    regex: Regex,
}

/// Result of [`Redactor::redact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    /// Scrubbed text (original secret values never appear in it).
    pub text: String,
    /// Total number of substitutions applied.
    pub count: usize,
    /// Rule ids / secret names that matched, in match order.
    pub hits: Vec<String>,
}

impl Redacted {
    /// Short chip label, e.g. "1 secret hidden" / "3 secrets hidden".
    pub fn chip(&self) -> String {
        match self.count {
            0 => String::new(),
            1 => "1 secret hidden".to_string(),
            n => format!("{n} secrets hidden"),
        }
    }
}

/// Secret scrubber. On by default; never disabled for logs.
#[derive(Debug)]
pub struct Redactor {
    /// (name, value) known secrets from the `SecretStore`, `.env*` files, env vars.
    known: Vec<(String, String)>,
    patterns: Vec<Pattern>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    /// Redactor with the built-in pattern set and no known values.
    pub fn new() -> Self {
        Self {
            known: Vec::new(),
            patterns: default_patterns(),
        }
    }

    /// Register a known secret value. `name` appears in `[redacted:<NAME>]`.
    pub fn add_secret(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let value = value.into();
        if value.len() < 4 {
            return;
        }
        self.known.push((name.into(), value));
    }

    /// Register many known secrets at once.
    pub fn add_secrets<I, S1, S2>(&mut self, secrets: I)
    where
        I: IntoIterator<Item = (S1, S2)>,
        S1: Into<String>,
        S2: Into<String>,
    {
        for (n, v) in secrets {
            self.add_secret(n, v);
        }
    }

    pub fn known_count(&self) -> usize {
        self.known.len()
    }

    /// Scrub `text`. Layer 1 (known values) first, then layer 2 (patterns).
    pub fn redact(&self, text: &str) -> Redacted {
        let mut out = text.to_string();
        let mut hits: Vec<String> = Vec::new();

        // Layer 1: exact known values via Aho-Corasick.
        if !self.known.is_empty() {
            let values: Vec<&str> = self.known.iter().map(|(_, v)| v.as_str()).collect();
            if let Ok(ac) = AhoCorasick::new(values) {
                // Replace from the end so byte offsets stay valid.
                let matches: Vec<_> = ac.find_iter(&out).collect();
                for m in matches.iter().rev() {
                    let idx = m.pattern().as_usize();
                    let name = &self.known[idx].0;
                    out.replace_range(m.range(), &format!("[redacted:{name}]"));
                    hits.push(name.clone());
                }
            }
        }

        // Layer 2: generic patterns.
        for p in &self.patterns {
            let rule = p.rule_id.to_string();
            let replacement = format!("[redacted:{rule}]");
            let n = out.matches(replacement.as_str()).count();
            let _ = n;
            let new_out = p.regex.replace_all(&out, replacement.as_str());
            if new_out != out {
                // Count occurrences added by this rule.
                let before = hits.len();
                let added = new_out.matches(replacement.as_str()).count()
                    - out.matches(replacement.as_str()).count();
                for _ in 0..added {
                    hits.push(rule.clone());
                }
                let _ = before;
                out = new_out.into_owned();
            }
        }

        let count = hits.len();
        Redacted {
            text: out,
            count,
            hits,
        }
    }

    /// Convenience: true when [`redact`](Self::redact) would change the text.
    pub fn contains_secret(&self, text: &str) -> bool {
        self.redact(text).count > 0
    }
}

/// Scrub with only the built-in patterns (no known values).
pub fn redact_text(text: &str) -> Redacted {
    Redactor::new().redact(text)
}

/// Default deny rules (ARCHITECTURE.md §6): paths agents may not read at all.
/// The user can override per path with a visible warning (checked elsewhere).
pub fn is_denied_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let full = path.to_string_lossy().to_lowercase();

    // File-name rules.
    if name.starts_with(".env") {
        return true;
    }
    if name.ends_with(".pem") || name.ends_with(".key") {
        return true;
    }
    if name.starts_with("id_rsa") || name.starts_with("id_ed25519") {
        return true;
    }
    if name == "credentials.json" || name == "keychain" {
        return true;
    }

    // Directory rules.
    for seg in [
        ".ssh",
        ".aws",
        ".gnupg",
        "keychains",
        "keychain",
        ".pki",
        ".docker",
    ] {
        if full.split(['/', '\\']).any(|part| part == seg) {
            return true;
        }
    }
    // Browser profile folders.
    for seg in [
        "chrome",
        "chromedefault",
        "firefox",
        "profiles",
        "browser",
        "edge",
        "brave-browser",
    ] {
        if full.contains(seg) && full.contains("profile") {
            return true;
        }
    }
    if full.contains("google-chrome") || full.contains("chromium") {
        return true;
    }
    false
}

fn default_patterns() -> Vec<Pattern> {
    let rules: &[(&str, &str)] = &[
        ("anthropic-key", r"sk-ant-[A-Za-z0-9_\-]{8,}"),
        ("openai-key", r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}"),
        ("github-token", r"gh[pousr]_[A-Za-z0-9]{20,}"),
        ("aws-access-key", r"AKIA[0-9A-Z]{16}"),
        (
            "aws-secret-key",
            r#"(?i)aws_secret_access_key['\"\s:=]+[A-Za-z0-9/+=]{20,}"#,
        ),
        ("stripe-key", r"(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{10,}"),
        ("slack-token", r"xox[baprs]-[A-Za-z0-9\-]{8,}"),
        ("google-api-key", r"AIza[0-9A-Za-z_\-]{30,}"),
        ("private-key", r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
        (
            "jwt",
            r"eyJ[A-Za-z0-9_\-]+\.eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-=.]+",
        ),
        (
            "connection-string",
            r#"(?i)(?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis)://[^\s'\";]+"#,
        ),
        (
            "generic-assignment",
            r#"(?i)(api[_-]?key|auth[_-]?token|access[_-]?token|secret|passwd|password|bearer)\s*[:=]\s*['"]?[^\s'";]{6,}"#,
        ),
    ];
    rules
        .iter()
        .filter_map(|(id, re)| {
            Regex::new(re)
                .ok()
                .map(|regex| Pattern { rule_id: id, regex })
        })
        .collect()
}

/// Documented fake-secret fixtures: one fake key per major provider.
/// Every model-bound / stored path must fully redact each of these.
/// These are **fake** values reserved for tests — never real credentials.
pub mod fixtures {
    pub const OPENAI: (&str, &str) = (
        "OPENAI_API_KEY",
        "sk-proj-f4k3T3stOpenAIKey0123456789abcdefXYZ",
    );
    pub const ANTHROPIC: (&str, &str) = (
        "ANTHROPIC_API_KEY",
        "sk-ant-f4k3T3st-Anthr0pic-Key-0123456789ab",
    );
    pub const GITHUB: (&str, &str) = ("GITHUB_TOKEN", "ghp_f4k3T3stT0ken0123456789abcdefgh");
    pub const AWS_ACCESS: (&str, &str) = ("AWS_ACCESS_KEY_ID", "AKIAF4K3T3STEX4MPLE0");
    pub const STRIPE: (&str, &str) = ("STRIPE_SECRET_KEY", "sk_test_f4k3T3stStr1peKey0123456789");
    pub const SLACK: (&str, &str) = ("SLACK_TOKEN", "xoxb-f4k3T3st-0123456789-abcdefghij");
    pub const GOOGLE: (&str, &str) = ("GOOGLE_API_KEY", "AIzaF4k3T3stG00gleAp1K3y0123456789abcde");
    pub const JWT: (&str, &str) = ("JWT", "eyJmYWtlfQ.eyJ0ZXN0fQ.ZmFrZXNpZ25hdHVyZXhhbXBsZQ");
    pub const PRIVATE_KEY: (&str, &str) = (
        "PRIVATE_KEY",
        "-----BEGIN FAKE PRIVATE KEY-----\nf4k3\n-----END FAKE PRIVATE KEY-----",
    );
    pub const CONNECTION_STRING: (&str, &str) = (
        "DATABASE_URL",
        "postgres://fak3user:f4k3pass@localhost:5432/fakedb",
    );
    pub const GENERIC: (&str, &str) = ("api_key", "api_key=f4k3T3stGenericSecretValue99");

    /// All fixture (name, value) pairs.
    pub fn all() -> Vec<(&'static str, &'static str)> {
        vec![
            OPENAI,
            ANTHROPIC,
            GITHUB,
            AWS_ACCESS,
            STRIPE,
            SLACK,
            GOOGLE,
            JWT,
            PRIVATE_KEY,
            CONNECTION_STRING,
            GENERIC,
        ]
    }

    /// All fixture values only.
    pub fn values() -> Vec<&'static str> {
        all().into_iter().map(|(_, v)| v).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures;
    use super::is_denied_path;
    use super::Redactor;
    use std::path::Path;

    fn redactor_with_fixtures() -> Redactor {
        let mut r = Redactor::new();
        r.add_secrets(fixtures::all());
        r
    }

    #[test]
    fn redact_fake_secret_corpus_fully_scrubbed() {
        // Every documented fixture must be fully redacted on this path.
        let r = redactor_with_fixtures();
        for (name, value) in fixtures::all() {
            let text = format!("calling with {value} inside a long sentence about research");
            let out = r.redact(&text);
            assert!(out.count >= 1, "fixture {name} was not redacted");
            assert!(
                !out.text.contains(value),
                "fixture {name} leaked into redacted text"
            );
            assert!(
                out.text.contains("[redacted:"),
                "fixture {name} missing marker"
            );
        }
    }

    #[test]
    fn redact_patterns_catch_unregistered_secrets() {
        // Layer 2 must catch provider-shaped keys even when not registered.
        let r = Redactor::new();
        let samples = [
            "sk-ant-fresh-Unregistered-Key-9876543210",
            "ghp_UnregisteredT0ken0123456789abcdef",
            "AKIAUNREGISTEREDXX00",
            "sk_live_unregisteredStripeKey99",
        ];
        for s in samples {
            let out = r.redact(&format!("token {s} here"));
            assert!(out.count >= 1, "pattern missed {s}");
            assert!(!out.text.contains(s), "pattern leaked {s}");
        }
    }

    #[test]
    fn redact_known_values_use_names() {
        let mut r = Redactor::new();
        r.add_secret("MY_PROVIDER", "super-secret-value-xyz-12345");
        let out = r.redact("key is super-secret-value-xyz-12345 ok");
        assert_eq!(out.count, 1);
        assert!(out.text.contains("[redacted:MY_PROVIDER]"));
        assert!(!out.text.contains("super-secret-value-xyz-12345"));
        assert_eq!(out.chip(), "1 secret hidden");
    }

    #[test]
    fn redact_tool_args_audit_log_and_error_paths() {
        // Tool-call args, audit rows, error messages and memory writes share this path.
        let r = redactor_with_fixtures();
        let args = serde_json::json!({
            "query": "research",
            "key": fixtures::OPENAI.1,
            "note": format!("see {}", fixtures::GITHUB.1),
        });
        let args_str = args.to_string();
        let out = r.redact(&args_str);
        assert!(!out.text.contains(fixtures::OPENAI.1));
        assert!(!out.text.contains(fixtures::GITHUB.1));
        let err = format!("request failed with {}", fixtures::STRIPE.1);
        let out = r.redact(&err);
        assert!(!out.text.contains(fixtures::STRIPE.1));
        assert_eq!(r.redact("nothing secret here").count, 0);
    }

    #[test]
    fn redact_deny_paths_blocked() {
        for p in [
            "/home/u/proj/.env",
            "/home/u/proj/.env.local",
            "/home/u/key.pem",
            "/home/u/id_rsa",
            "/home/u/.ssh/config",
            "/home/u/.aws/credentials",
        ] {
            assert!(is_denied_path(Path::new(p)), "should deny {p}");
        }
        assert!(!is_denied_path(Path::new("/home/u/proj/reports/out.md")));
        assert!(!is_denied_path(Path::new("/home/u/proj/files/notes.md")));
    }

    #[test]
    fn redact_short_values_ignored_and_empty_safe() {
        let mut r = Redactor::new();
        r.add_secret("X", "ab");
        assert_eq!(r.known_count(), 0);
        let out = r.redact("");
        assert_eq!(out.count, 0);
        assert_eq!(out.text, "");
    }
}
