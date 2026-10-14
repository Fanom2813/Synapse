# Setup

## Toolchain
- Rust stable (see `rust-toolchain.toml`). `cargo packager` needs no install: `.cargo/config.toml` aliases it to the in-workspace `synapse-packager` crate, which runs the real cargo-packager CLI (pinned in `[workspace.dependencies]`). Unsigned local packaging: `cargo packager --release` (see `Packager.toml`).
- macOS: Xcode command line tools. Windows: MSVC build tools. Linux (Debian/Ubuntu):
  `sudo apt install build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig-dev libfreetype-dev libssl-dev libx11-xcb-dev libxcb1-dev libdbus-1-dev`
  (match GPUI Kit's CI list if it differs). Tray on GNOME needs the AppIndicator extension.

## Keys (all optional; nothing is required to build or test)
Tests use mocks. These are only for `SYNAPSE_LIVE=1` smoke tests and real use; in the app they are
entered in Settings and stored in the OS keychain.

| Variable | Used for |
|---|---|
| `SYNAPSE_LIVE` | `1` enables live smoke tests against real services |
| `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY` | live provider smoke tests (any one) |
| `SEARXNG_URL` | live research smoke test with a SearXNG instance (JSON format enabled) |
| `BRAVE_API_KEY`, `TAVILY_API_KEY`, `EXA_API_KEY`, `JINA_API_KEY` | optional search providers |
| `PACKAGER_PRIVATE_KEY`, `PACKAGER_PRIVATE_KEY_PASSWORD` (repo secrets) | signed release builds + updater signatures; without them the release workflow still ships unsigned installers |

Local models: run Ollama (`:11434`) or LM Studio (`:1234`); Synapse detects them.
CLI agents: install and log in to `claude`, `codex`, `opencode` or `gemini` as usual; Synapse detects them.
- [ ] Provider API credentials (for example OPENAI_API_KEY, ANTHROPIC_API_KEY, GEMINI_API_KEY, or OPENROUTER_API_KEY): optional for live use/tests; users can instead enter provider keys in the app or use a local model/logged-in CLI. Tests use mocks.
- [ ] Optional research credentials (BRAVE_API_KEY, TAVILY_API_KEY, EXA_API_KEY, JINA_API_KEY) or SEARXNG_URL: enable those specific search backends; keyless search remains the default.
- [ ] PACKAGER_PRIVATE_KEY (+ PACKAGER_PRIVATE_KEY_PASSWORD): only for signed release distribution with verified auto-updates, not local builds or tests. Generate with `cargo packager signer --help`; the public key ships in the app's update config. In-app update checks default to the `latest.json` manifest on GitHub Releases; override locally with `SYNAPSE_UPDATE_ENDPOINT`.
