# Synapse: stack (glue, don't build)

Versions checked 2026-10-14 (see research/gpui.md and research/rust-stack.md). Pin **exactly** (`=x.y.z`)
in `[workspace.dependencies]` for anything pre-1.0 that churns (GPUI Kit, rig/genai, rmcp, ACP).
`Cargo.lock` is committed. Upgrade on purpose, one dependency per commit, with the full check green.

Before writing any non-trivial component or algorithm, look for it in this table, in GPUI Kit's
component list and examples, and on crates.io. Writing it ourselves is the last resort.

## UI

| Need | Pick | Notes |
|---|---|---|
| UI framework | **GPUI** via **`gpui-kit = "=0.7.3"`** (longbridge/gpui-kit, Apache-2.0) | Never depend on `gpui` directly; use the kit's re-export (`gpui_kit::gpui`, which is `gpui-pre =0.3.9`, a snapshot of zed@93d7456). Every GPUI-touching crate must resolve to the same `gpui-pre`. Not crates.io `gpui` 0.2.2 (stale), not zed git, not gpui-ce (incompatible). |
| Components | GPUI Kit: chat message/bubble/scroller, markdown (streaming, word fade-in; see its `stream-markdown` example), code editor + tree-sitter, dock/tabs/resizable panels, sidebar, dialogs/sheets, toasts, forms/settings, command palette, virtual list/table, charts, skeleton/shimmer, animation (spring/keyframes/stagger), theme | Enable only needed tree-sitter languages (rust, python, js/ts, json, bash, markdown). |
| Faster renderer (optional) | GPUI Kit feature `gpui-fast` behind our cargo feature `fast` | Retained-mode fork; experimental. Benchmark both (PERFORMANCE.md). |
| Perf overlay | `gpui-fps` (from the kit) | Dev builds only, toggle with a hidden shortcut. |
| Tray | `gpui-tray = "0.1"` with features `gpui-kit`, `menu-state` | Runs on GPUI's loop, no GTK. Fallback: `tray-icon` 0.26 + `muda` (KSNI backend on Linux). |
| Global hotkey | `global-hotkey = "0.8"` | Wayland: XDG GlobalShortcuts portal via `ashpd`. |
| Notifications | GPUI `SystemNotification` | Fallback `notify-rust` 4 on Linux if needed. |
| Launch at login | `auto-launch = "0.6"` | macOS 13+: `smappservice-rs` if auto-launch misbehaves. |
| Single instance + IPC | `interprocess = "2"` local sockets | |
| Graph view | GPUI `canvas` + simple force layout (`fdg` or a 100-line spring layout) | Agents are few (≤ 20 nodes); no graph library needed. |

## Core / I/O

| Need | Pick | Notes |
|---|---|---|
| Async runtime | `tokio` 1.x, 2 worker threads | Bridge copied from Zed's `gpui_tokio` (Apache-2.0, ~100 lines, keep the license header). |
| Cancellation | `tokio-util` `CancellationToken` | |
| HTTP | `reqwest` (rustls, json, stream), `eventsource-stream` if needed | One shared client. |
| Provider catalog | **models.dev `api.json`** | Embed a trimmed gz snapshot in `assets/`; refresh daily with `If-None-Match`; cache in SQLite. |
| LLM client | **`genai`** (jeremychone, MIT/Apache-2.0), pinned exact | 36 adapters, custom OpenAI-compatible endpoints, Ollama, streaming tool calls, provider-native web search tools. Wrapped behind our `ModelBackend` trait. Runner-up: `rig-core =0.44.0` (has a models.dev catalog; heavy churn). Raw fallback: `async-openai`. |
| CLI agents | **`agent-client-protocol` 3.3.x** (Apache-2.0) + ACP registry JSON (`cdn.agentclientprotocol.com/registry/v1/latest/registry.json`) | Claude Code: `npx @agentclientprotocol/claude-agent-acp`; Codex: `@agentclientprotocol/codex-acp`; `opencode acp`; `gemini --acp`; `goose acp`; `copilot --acp`; `cursor-agent acp`. Headless fallbacks: `claude -p --output-format stream-json --verbose`, `codex exec --json`, `opencode run --format json`, `gemini -p --output-format stream-json`. Resolve PATH from the user's login shell (GUI apps don't inherit it). |
| MCP | **`rmcp` 3.5.x** features `client`, `server`, `transport-child-process`, `transport-streamable-http-client-reqwest`, `auth` | Registry: `registry.modelcontextprotocol.io/v0.1/servers`. |
| Orchestration | **Own code** in `synapse-core` (~1–2k lines) on tokio channels | Patterns: Anthropic multi-agent research system, AutoGen SelectorGroupChat, STORM/Co-STORM, multi-agent debate. See TEAM-PROTOCOL.md. |
| Deep research | Port **dzhng/deep-research** (MIT) loop + **STORM** perspectives | See DEEP-RESEARCH.md. |
| Search providers + fusion | Port from **paperfoot/search-cli** (crate `agent-search` 0.9, MIT): its 13 provider adapters (Brave, Serper, Exa, Linkup, Jina, Firecrawl, Tavily, SerpApi, Perplexity, Parallel, xAI, ...) and reciprocal-rank-fusion merge | It is bin-only (no lib target), so copy the modules with the MIT notice; don't depend on it. If the `search` binary (search-cli) or `ketch` (Go, MIT) is on PATH, also offer it as an external provider via its JSON output. |
| HTML → markdown | `dom_smoothie` 0.18 (Readability port, MIT) → `htmd` 0.5 (Apache-2.0) | Never `html2md` (GPL). Fallback: Jina Reader `r.jina.ai` (keyless, 20 RPM). |
| PDF text | `pdf-extract` | Or Jina Reader. |
| robots / rate limit | `texting_robots`, `governor` 0.10 | Per-host limiter. |
| Headless browser | none by default; `chromiumoxide` 0.9 opt-in later, driving the user's own Chrome | Never bundle Chromium. |
| Storage | `rusqlite =0.40.x` feature `bundled` (FTS5) + `rusqlite_migration` + `sqlite-vec` 0.1.9 | |
| Embeddings | `model2vec-rs` 0.3 (static embeddings, no ONNX, tiny) | Lazy-loaded on first semantic search; model file downloaded once to the data dir. |
| Personas | Markdown + YAML frontmatter via `gray_matter` or `serde_yaml_ng` | Sources: msitarzewski/agency-agents (MIT, ~300), wshobson/agents (MIT, 202 agents + 184 skills), VoltAgent/awesome-claude-code-subagents (MIT, ~170), 0xfurai/claude-code-subagents (MIT). contains-studio/agents has no license: link only, never bundle. |
| Permissions | own rules engine + `globset` | Copy Claude Code's allow/ask/deny semantics (deny > ask > allow). |
| Shell sandbox | `nono` 0.80 (Seatbelt / Landlock) | Windows: no sandbox, always ask. |
| Diffs | `similar` | For write/edit previews and artifact changelogs. |
| Secrets | `keyring` 4.2 (or GPUI credentials API), `secrecy` | |
| Schemas | `schemars` 1.x, `serde`, `serde_json` | Tool parameter schemas. |
| Config | `toml` + serde | (figment is stale.) |
| Logging | `tracing`, `tracing-subscriber`, `tracing-appender` (rolling file) + redaction layer | |
| Tokens | provider-reported usage first; `tiktoken-rs` / `bpe-openai` only for estimates | |
| Schedules | `croner` (cron parsing) + own scheduler task (sleeps until next due, no polling) | |
| Packaging/update | `cargo-packager` + `cargo-packager-updater` | .app/.dmg, .msi/NSIS, AppImage/.deb. |

## Reference code to port from (not dependencies)

| Project | License | Port |
|---|---|---|
| **thClaws/thClaws** (Rust agent harness, wry/tao GUI) | MIT OR Apache-2.0 | Shell confinement (Seatbelt / Landlock / bubblewrap fallback) if `nono` falls short; provider adapters; MCP wiring; multi-agent supervision ideas. Not a base to fork: different UI stack and product. |
| **zed-industries/zed** `gpui_tokio`, ACP client code | Apache-2.0 parts only (Zed is mostly GPL: never copy GPL crates) | tokio bridge; how Zed spawns ACP agents |
| **paperfoot/search-cli** | MIT | search providers, RRF |
| **dzhng/deep-research**, **stanford-oval/storm** | MIT | research loop, perspectives |

## Dev / test

| Need | Pick |
|---|---|
| UI tests | GPUI `TestAppContext` (`#[gpui::test]`, `test-support` feature) |
| HTTP mocks | `wiremock` |
| Snapshot tests | `insta` |
| Process RSS/CPU in perf tests | `sysinfo` or `memory-stats` |
| Lint/format | `cargo fmt`, `cargo clippy -D warnings` |
| CI | GitHub Actions matrix: macos-latest, windows-latest, ubuntu-latest (Linux deps: libxkbcommon, libwayland, libvulkan, fontconfig, etc. per GPUI Kit's CI) |
