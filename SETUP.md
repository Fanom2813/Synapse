# Setup

## Build host (where sg builds Synapse)

The unattended loop runs on a **headless Linux VPS** (Ubuntu 24.04, x86_64, 3 vCPU, 8 GB RAM, 4 GB swap),
reached over Tailscale only. There is no display, no GPU and no macOS/Windows there, so:

- Everything a task's `verify:` or a milestone's `accept:` runs must pass **headless on Linux**: unit and
  integration tests, GPUI views through `TestAppContext` (no window needed), mocks for every service.
- Tests that need a real window, GPU, macOS or Windows (perf probes with real rendering, tray, keychain,
  Seatbelt sandbox, notifications, global hotkeys) are marked `#[ignore = "needs-desktop"]` (or
  `#[cfg(target_os = ...)]`) and run in **GitHub Actions** (macOS, Windows, Ubuntu with Xvfb) and on the owner's Mac.
- Smoke-running the real binary on the VPS uses `xvfb-run` with Mesa's software Vulkan (lavapipe); it proves
  the window opens and the app does not crash, not performance.

Host setup (once, as root on the VPS):
```sh
apt install -y build-essential pkg-config clang mold cmake libssl-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig1-dev libfreetype-dev \
  libx11-xcb-dev libxcb1-dev libdbus-1-dev libsecret-1-dev xvfb mesa-vulkan-drivers gh
curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal -c clippy,rustfmt
gh auth login            # sg needs gh for GitHub milestones/issues
sg key                   # model keys for the loop
```
(Match GPUI Kit's own Linux CI package list if it differs.)

Build settings for an 8 GB machine (in `.cargo/config.toml` and the dev profile, committed):
- linker `mold` via clang (`-C link-arg=-fuse-ld=mold`); `CARGO_BUILD_JOBS=2` if builds hit OOM.
- `[profile.dev] debug = "line-tables-only"`, `[profile.dev.package."*"] opt-level = 1, debug = false`
  (smaller target dir, less RAM, faster tests); `[profile.test]` inherits dev.
- The release profile (`lto = "fat"`) is only built for perf/packaging tasks, never for ordinary task checks.
- Watch disk: GPUI target dirs grow large; `cargo sweep`/`cargo clean -p` old artifacts if free space < 15 GB.


## Toolchain
- Rust stable (see `rust-toolchain.toml`), `cargo-packager` (`cargo install cargo-packager --locked`) for installers.
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
| `APPLE_SIGNING_IDENTITY`, `APPLE_NOTARY_PROFILE`, `WINDOWS_CERT_*`, `PACKAGER_UPDATER_KEY` | signed release builds only |

Local models: run Ollama (`:11434`) or LM Studio (`:1234`); Synapse detects them.
CLI agents: install and log in to `claude`, `codex`, `opencode` or `gemini` as usual; Synapse detects them.
