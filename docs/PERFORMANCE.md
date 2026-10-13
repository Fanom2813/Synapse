# Synapse: performance budgets

The owner's #1 requirement: the user should not feel that Synapse is running.

## 1. Budgets (release build, measured by `synapse-acceptance` perf tests and the `--perf-probe` flag)

| Scenario | CPU | Memory (physical footprint / RSS) |
|---|---|---|
| Window visible, nothing happening, after 5 s settle | ≤ 0.5 % avg over 30 s, ≈ 0 wakeups when nothing animates | ≤ 120 MB |
| Window closed, tray only, no run (window destroyed) | ≈ 0 % | ≤ 60 MB |
| Team run with 5 agents streaming, window visible | ≤ 15 % of one core | ≤ 250 MB |
| Scrolling a 5,000-message room | 60 fps, no frame > 16 ms on a 2020 laptop | +≤ 30 MB over idle |
| Cold start to interactive window | ≤ 600 ms | |
| Binary size (stripped, per platform) | | ≤ 40 MB |

Plain GPUI is ~26 MB footprint / ~57 MB RSS for hello world, so these are achievable but must be watched.

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
  marked `#[ignore]` on CI runners without a GPU and run locally / on a macOS runner).
- Dev builds show the GPUI Kit `gpui-fps` overlay (toggle Cmd/Ctrl-Alt-Shift-P).
- Known risk to check early: whether GPUI still wakes every display refresh while idle on macOS/X11.
  If yes, document it and pick the backend (`fast` or not) that is idle-quiet.
