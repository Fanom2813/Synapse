# Synapse perf results (PERFORMANCE.md §1, M10.4)

Measured headless with `synapse --perf-probe <scenario> --seconds N`
(release build) plus the `m10_performance` acceptance probes. Toolkit-free
path: the GPUI shell wires `IdleFramePolicy::needs_frame` into its render
loop; numbers below cover the logic + process dimensions measurable
without a display. macOS footprint / 2020-laptop frame numbers need a
device run before the release gate closes.

Date: 2026-10-14. Host: Linux CI container (this session).
Build: `cargo test -p synapse-acceptance --release --test m10_performance`.

| Scenario | Result | CPU (avg) | Memory | Frames | Budget | Verdict |
|---|---|---|---|---|---|---|
| Team run, 5 agents streaming (`run5`) | 30 frames × 5 agents × 20 deltas = 3,000 deltas → 5 updates/frame | burst < 5 s wall (< 1 frame budget/frame); probe avg ≈ 0% | probe peak RSS ≈ test-harness RSS, well under 250 MB cap | 5 notifies/frame max (coalesced) | 15% of one core / 250 MB | PASS headless |
| Scroll 5,000-message room (`scroll`) | 5,000 rows, 30-row visible window at 4 fling offsets | — (layout proxy, no GPU) | window delta < 1 KB (bodies only), cap +30 MB | slowest window ≪ 16 ms (µs on CI) | 60 fps, no frame > 16 ms / +30 MB | PASS headless |
| `run5` probe report | `run_probe(Run5, 0)` passed | under 15% cap | under 250 MB cap | streaming frames expected | same | PASS |
| `scroll` probe report | `run_probe(Scroll, 0)` passed, `max_frame_ms` < 16 | under cap | window delta under cap | `max_frame_ms` reported | same | PASS |

Regressions fixed this pass: none in app code — the `run5`/`scroll`
scenarios did not exist yet, so the probe only covered `idle`/`tray`.
Added `ProbeScenario::{Run5, Scroll}` with the §1 caps
(15% / 250 MB, 16 ms / +30 MB), the coalesced-burst and visible-window
exercises over the real `TeamRoomState` / `ChatState` paths, and the
`m10_performance` release probes above.

Still to measure on device (assumed: headless numbers do not replace
them): macOS footprint (idle 120 MB / tray 60 MB caps), cold-start
300 ms target, stripped binary ≤ 30 MB (`stat target/release/synapse`),
installer ≤ 20 MB, and a 2020-laptop 60 fps scroll with the GPU-backed
GPUI view. `fast` vs default feature comparison likewise needs the
device run.
