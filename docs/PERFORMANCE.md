# Synapse: performance budgets

The owner's #1 requirement: the user should not feel that Synapse is running.

## 1. Budgets (release build)

Two numbers per row: the **target** we design for and the **hard cap** that fails CI / the milestone.

| Scenario | CPU target / cap | Memory target / cap (macOS footprint, Linux/Windows private bytes) |
|---|---|---|
| Window visible, nothing happening, after 5 s settle | ~0 % (no wakeups while nothing animates) / 0.5 % avg over 30 s | 70 MB / 120 MB |
| Window closed, tray only, no run (window destroyed) | 0 % / 0.2 % | 35 MB / 60 MB |
| Team run, 5 agents streaming, window visible | 5 % / 15 % of one core | 150 MB / 250 MB |
| Scrolling a 5,000-message room | 60 fps, no frame > 16 ms on a 2020 laptop | +20 MB / +30 MB over idle |
| Cold start to interactive window | 300 ms / 600 ms | |
| App binary (stripped, per platform) | 20 MB / 30 MB | |
| Installer download (dmg / msi / AppImage) | 12 MB / 20 MB | |

Plain GPUI is ~26 MB footprint / ~57 MB RSS for hello world. Every new dependency, feature flag or asset
must be justified against these numbers.

## 1b. Keeping the app small on disk

- Release profile: `opt-level = 3` (speed matters for UI), `lto = "fat"`, `codegen-units = 1`,
  `strip = "symbols"`, `panic = "abort"`, `debug = false`. Try `opt-level = "s"` for non-UI crates if it
  measures smaller without hurting the perf probes.
- `default-features = false` on every dependency; enable only what we use. One TLS stack for the whole tree:
  `rustls` with the `ring` provider (not `aws-lc-rs`), and no OpenSSL. One HTTP client (`reqwest`), one async
  runtime (`tokio`), one version of each crate: `cargo tree -d` must show no duplicate versions of heavy
  crates (reqwest, hyper, tokio, rustls, serde_json, image).
- Fonts: only the four subset variable fonts from UI.md §6 (≤ 700 KB total); no other bundled fonts.
  No bundled Chromium, no ONNX runtime, no ICU data.
- Assets: SVG over PNG; avatars are SVG; icons: only the Phosphor SVGs we use (UI.md §7), never two icon sets; the models.dev snapshot is
  trimmed to the fields we use and gz-compressed; tree-sitter grammars only for the languages listed in STACK.md.
- Downloaded on first use instead of bundled: the embeddings model (model2vec), DiceBear avatar packs beyond
  the default set, persona collections, MCP servers, ACP adapters.
- Heavy optional parts behind cargo features that are **on** in the release build but can be measured
  separately: `browser` (chromiumoxide), `sandbox` (nono), `vectors` (sqlite-vec + model2vec), `fast` (gpui-fast).
- `cargo bloat --release --crates` report from the perf task on the build box; a size regression > 5 % or over the cap fails.

## 2. Rules

1. **No polling.** No `loop { sleep }` and no interval timers for UI state. Channels, `Notify`, OS events.
   The schedule runner sleeps until the next due time.
2. **Animations only when visible and meaningful** (UI.md §4). An infinite GPUI animation keeps the
   app redrawing at the display refresh rate, so idle avatars settle into a still pose.
3. **Coalesce streaming**: text deltas from all agents are batched per frame; one `cx.notify()` per
   changed message per frame at most. Never notify the whole room for a single delta.
4. **Virtualise every long list** (GPUI Kit virtual list / message scroller). Off-screen markdown is not laid out.
5. **Destroy, don't hide**: the main window is dropped after 60 s hidden. Caches (images, highlighted
   code, embeddings model) are dropped with it or after inactivity.
6. **Small tokio**: 2 worker threads; blocking work (`rusqlite`, PDF parsing, embeddings) on
   `spawn_blocking` with a cap.
7. **Lazy everything**: embeddings model, tree-sitter grammars, MCP servers, CLI agents, and the
   catalog parse start only when first needed. Child processes are killed after 5 min idle.
8. **Bounded memory**: message bodies are loaded per visible window from SQLite; source texts are
   never all held in memory; caches have LRU limits.
9. **One HTTP client**, connection reuse, gzip; catalog refresh at most daily with ETag.
10. Enable only needed GPUI Kit features and tree-sitter languages; `opt-level=3`, `lto="thin"`,
    `codegen-units=1`, `strip=true`, `panic="abort"` in release.
11. Try GPUI Kit's `gpui-fast` (cargo feature `fast`) and keep whichever measures better.

## 3. Measuring

- `synapse --perf-probe <scenario> --seconds 30` starts the app with mocks, runs the scenario
  (`idle`, `tray`, `run5`, `scroll`), and prints JSON: avg CPU %, peak/avg RSS, footprint (macOS),
  wakeups if available, frame times. Implemented with `sysinfo` / `memory-stats` reading its own process.
- Perf tests in `synapse-acceptance/tests/m10_performance.rs` assert the table above (run in release,
  marked `#[ignore]` where there is no GPU and run on the build box / locally).
- Dev builds show the GPUI Kit `gpui-fps` overlay (toggle Cmd/Ctrl-Alt-Shift-P).
- Known risk to check early: whether GPUI still wakes every display refresh while idle on macOS/X11.
  If yes, document it and pick the backend (`fast` or not) that is idle-quiet.
