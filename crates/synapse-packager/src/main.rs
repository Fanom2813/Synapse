// Copyright 2026 Synapse Contributors.
// SPDX-License-Identifier: MIT

//! `cargo packager` entry point that needs no `cargo install`.
//!
//! It forwards every argument to the real [`cargo_packager`] CLI (same crate
//! version the release workflow would install), so `cargo packager --help`
//! and `cargo packager --release` behave exactly like the upstream tool,
//! including unsigned local builds (signing only happens with `-k`).
//! Wired up via the `packager` alias in `.cargo/config.toml`.

use std::{env::args_os, ffi::OsStr, path::Path, process::exit};

fn main() {
    let mut args = args_os().peekable();
    let bin_name = match args
        .next()
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_stem)
        .and_then(OsStr::to_str)
    {
        Some("synapse-packager") => {
            if args.peek().and_then(|s| s.to_str()) == Some("packager") {
                // Invoked as `cargo packager ...`: drop the extra subcommand arg,
                // exactly like the upstream `cargo-packager` binary does.
                args.next();
                Some("cargo packager".into())
            } else {
                Some("cargo-packager".into())
            }
        }
        Some(stem) => Some(stem.to_string()),
        None => {
            eprintln!("synapse-packager wrapper unable to read first argument");
            exit(1);
        }
    };

    cargo_packager::cli::run(args, bin_name);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The shipped `Packager.toml` must parse with the real packager and
    /// cover every documented installer (macOS app/dmg, Windows nsis/wix,
    /// Linux appimage/deb) with no signing required for local builds.
    #[test]
    fn packager_config_covers_all_platforms_unsigned() {
        let text = std::fs::read_to_string(repo_root().join("Packager.toml"))
            .expect("Packager.toml exists at the repo root");
        let config: cargo_packager::Config =
            toml::from_str(&text).expect("Packager.toml parses as a packager Config");

        assert_eq!(config.product_name, "Synapse");
        assert!(
            config
                .identifier
                .as_deref()
                .unwrap_or_default()
                .contains('.'),
            "reverse-DNS identifier is set"
        );
        assert!(
            config.binaries.iter().any(|b| b.main),
            "one main binary is packaged"
        );
        assert!(config.deb.is_some(), "deb config present");
        assert!(config.appimage.is_some(), "appimage config present");
        assert!(config.dmg.is_some(), "dmg config present");
        assert!(config.nsis.is_some(), "nsis config present");
        assert!(config.wix.is_some(), "wix config present");

        let icons = config.icons.unwrap_or_default();
        assert!(!icons.is_empty(), "icons are listed");
        for icon in &icons {
            assert!(
                repo_root().join(icon).exists(),
                "packaged icon exists: {icon}"
            );
        }
    }
}
