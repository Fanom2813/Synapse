# GPUI flavors and the Synapse stack: research report (2026-10-14)

Method: GitHub API (`gh`), crates.io API, shallow clones of `longbridge/gpui-kit` and `gpui-ce/gpui-ce`, the `gpui-pre 0.3.9` crate source tarball, and web search. Nothing was built.
Clones: `scratchpad/gpui/{gpui-kit,gpui-ce,gpui-pre}`.

## TL;DR recommendation

- **GPUI source:** use `gpui-pre`, the crates.io snapshot of Zed's GPUI, pinned with `=` to the exact version that your `gpui-kit` release pins. Today `gpui-kit 0.7.3` pins `gpui-pre =0.3.9`, which is a snapshot of zed@93d7456 published 2026-10-12. Don't depend on it directly. Depend on `gpui-kit`, which re-exports `gpui`, and commit `Cargo.lock`.
- **Component kit:** use **GPUI Kit** (`gpui-kit = "0.7"`). This is the new name of longbridge/gpui-component; the crate `gpui-component` still exists as its styled layer. It has 17.2k stars, weekly releases and Apache-2.0. Longbridge Pro ships on it. It has chat-specific pieces: `MessageScroller` (virtual list that follows the tail), `Bubble`, `Message` and `Attachment`. It also has streaming Markdown with a "stream fade" word-reveal effect, a `stream-markdown` example, and a code editor using tree-sitter. Other pieces: dock/tabs/resizable panels, dialogs/sheets, toasts/notifications, settings forms, charts/plots, themes, `shimmer`, skeleton, spinner, motion primitives (springs/keyframes/stagger/presence), speech input, an optional Wry webview, and an FPS/CPU/memory HUD (`gpui-fps`).
- **Optional:** the `gpui-fast` feature switches the backend to Longbridge's performance fork with retained mode. Its numbers: −94% CPU per frame on an unchanged window and −73% on streaming text. It is marked "experimental", so turn it on behind a cargo feature and measure. Note that `gpui-webview` forces gpui-fast on.
- **Don't choose:**
  - `gpui 0.2.2` on crates.io: stale since 2025-10-22, and the upstream README instructions (`gpui_platform`) don't even resolve from crates.io.
  - A raw zed git dependency: it moves daily and is incompatible with the kit's types.
  - `gpui-ce`: a nice fork, but only 0.2.2 is stable, it's diverging, and gpui-kit can't use it.
  - `adabraka-gpui`: a single-author fork.

## 1. The GPUI flavors

| Flavor | What it is | Stars / activity | Latest | License | Verdict |
|---|---|---|---|---|---|
| **zed-industries/zed `crates/gpui`** (git) | Upstream. It is now split into `gpui`, `gpui_platform`, `gpui_macos`/`gpui_apple`, `gpui_windows`, `gpui_linux`, `gpui_wgpu`, `gpui_web` and `gpui_tokio` | 91.7k, pushed daily | HEAD | Apache-2.0 (gpui) | Source of truth, but breaking changes are frequent and it has no releases. Only useful for patches. |
| **`gpui` on crates.io** (owned by Zed) | Official release | 355k downloads | **0.2.2, 2025-10-22 (stale ~1 yr)** | Apache-2.0 | Avoid. It predates the platform-crate split, notifications, QuitMode and other recent work. |
| **`gpui-pre` (+26 `gpui-pre-*` crates)** | Snapshots of Zed's GPUI published to crates.io about weekly, each naming its zed commit (0.3.0 on 2026-09-03 through 0.3.9 on 2026-10-12). Published by huacnlee of Longbridge, not by the Zed org | ~145k downloads in 6 weeks | **0.3.9 = zed@93d7456** | Apache-2.0 | **Use it, through gpui-kit.** You get current upstream code as a semver-pinned crate. |
| **longbridge/gpui-fast** | Performance fork, "extracted from the Zed monorepo as a standalone workspace". Retained-mode layout, cached scroll tiles, native window composition (webview under GPUI overlays). Reuses the `gpui-pre-*` support crates | 210 stars, created 2026-10, pushed 2026-10-13 | 0.1.7 | Apache-2.0 | Optional, and the best perf story. It says "experimental". Use it through `gpui-kit` feature `gpui-fast`. |
| **gpui-ce/gpui-ce** (Community Edition) | Community fork with RFC governance. Adds CSS-like `.transitions()`, `backdrop_blur`, `rounded_smoothing`, a spring `motion` module, Parley text, wgpu on all platforms, system notifications, a keyring example, and its own `gpui_ce_tokio` | 1.35k stars, 151 forks, pushed 2026-10-13 | crates.io stable **0.2.2 (2026-08-28)**, plus `1.0.0-alpha.N` snapshots. Its release-tracking issue #278 was unresolved and was converted to a discussion on Oct 5 | MIT OR Apache-2.0 | Interesting for animation features, but "mostly API compatible, but that is changing". It's **incompatible with gpui-kit**, has no big component kit, and releases are stuck. |
| **adabraka-gpui** (Augani) | Fork with built-in tray, global hotkeys, notifications, auto-launch, single-instance and daemon mode | Small, single author, repo not reachable via API | 0.5.1 (2026-09-14) | Apache-2.0 | A tempting feature list, but it carries bus-factor risk and its own type universe. Use the separate crates instead. |
| `gpui-unofficial` | Third-party republish of Zed crates (1.24.1-pre) | low | — | — | Avoid. |
| **zed-industries/create-gpui-app** | Scaffolder | 389 stars, **last push 2025-04** | — | MIT | Stale. GPUI Kit's `examples/ai_recipes` and `hello_world` are better starting points. |
| **Zed's `ui` crate** | Zed's internal component crate | in the zed repo | not published | GPL-3.0 (Zed app crates are GPL) | **Don't depend on it.** It's GPL and tied to Zed's settings/theme crates. Read it for patterns only. |
| **awesome-gpui** (zed-industries) | Curated list | 1.4k stars, active | — | CC0 | Useful index (see §3). |

**How gpui-kit pins GPUI** (from `Cargo.toml`): `gpui = { package = "gpui-pre", version = "=0.3.9" }`, and the same exact pin for `gpui_platform`, `gpui_macros`, `sum-tree`, `reqwest_client`, plus a forked `reqwest = gpui-pre-reqwest =0.12.15`. The comment explains why. A caret requirement had let the weekly release move apps onto a snapshot that gpui-component didn't compile with (#3156), and `script/check-gpui-pin.ts` now fails CI on any pin that isn't exact. **Rule for Synapse:** let `gpui-kit` choose the gpui version, and never add your own `gpui-pre` version requirement. Any extra GPUI crates you add (tray, etc.) must resolve to the *same* gpui-pre version. A different source means distinct Rust types.

Renderers in the current upstream snapshot: macOS uses Metal. Windows uses DirectX 11 with DirectComposition and DirectWrite. Linux uses **wgpu** (the earlier Blade renderer is gone) on Wayland and X11. There is also a web backend (`gpui_web`, wasm). Zed itself has shipped GA on Windows and Linux, so both platforms are production-grade. Linux has the most rough edges: HiDPI on X11, IME, and tray, which has no GTK.

## 1b. GPUIX (https://gpuix.dev, remorses/gpuix): added at the user's request

- **What it is:** React and Solid bindings for GPUI. You write TSX, and a JS runtime (Bun by default, Node, or a smaller Hermes path) sends batched mutations through napi-rs to a retained Rust element tree that GPUI paints. There's no webview or DOM. It uses a **pinned GPUI fork as a git submodule** and owns its own type universe, so it can't be combined with gpui-kit's Rust components.
- **Repo:** 2.6k stars, 78 forks, 36 open issues, pushed 2026-10-11, Apache-2.0. npm releases: `@gpuix/react` 0.10.0 (2026-09-21) and `@gpuix/solid` 0.10.0. Pre-1.0: "a new release can break either package", so pin the adapter and `@gpuix/native` exactly.
- **Features:**
  - Primitives: `div`, `text`, `code` (highlighting), `diff`, **`markdown`**, input/textarea, **`virtual-list`**, `img`, `svg`.
  - Motion: **`motion.div` + `AnimatePresence`** in Framer-Motion style.
  - Headless Select, Combobox and Tooltip.
  - macOS menu bar, blurred or transparent windows, background launch via launchd and `GPUIX_BACKGROUND`, auto-update from GitHub Releases, file picker, AccessKit.
  - A WebGPU/WebGL2 browser build.
  - Hot reload with `bun --hot`, which makes iteration very fast.
- **Gaps for Synapse:** the README doesn't mention a **tray icon, global hotkeys or OS notifications**. Its component set is small compared with gpui-kit's 75+: no dock, settings, charts or dialogs kit. Linux ignores some window options, and nested scrolling isn't supported.
- **Footprint, from its own figures:**
  - Hello world: **47 MB footprint / 143 MB RSS against 26 MB / 57 MB for plain GPUI** (gpuix PR #603). The JS engine roughly doubles to triples memory.
  - Bundles: **82 MB** for the Bun chat `.app`, 34 MB for the Hermes counter, 12 MB + 22 MB sidecar for hermes-node.
  - Idle CPU: they fixed a 73% idle-CPU `setImmediate` loop down to ~1% with a paced macOS frame loop. That is still not 0%. It stops repaint on blur, and `collectGarbageWhenIdle()` exists because of JS GC.
- **Verdict:** this is the best "fast delivery for a TS/React team" option in the GPUI family. It's **not** the choice when the top priority is the lowest RAM/CPU: it costs ~2–3× the RAM and a much larger bundle. It also lacks tray, hotkeys and notifications, and it locks you to its fork. You could prototype the UI in GPUIX, but Synapse should be all Rust on gpui-kit. The agent backend (tokio, reqwest, rmcp) is Rust either way. GPUIX would force an IPC or napi boundary between the JS UI and the Rust agents.

## 2. GPUI Kit (longbridge/gpui-kit, formerly gpui-component)

- 17.2k stars, 1,055 forks, 113 open issues, pushed 2026-10-13. Releases are weekly: v0.6.4 (09-18) → v0.7.3 (10-12). Licensed Apache-2.0, docs under a separate license. Docs at https://gpui-kit.com plus docs.rs. It also ships agent "skills" for Claude Code and similar tools (`skills/`).
- Layers:
  - `gpui-base`: unstyled behavior, motion, text/markdown engine, a11y.
  - `gpui-component`: styled, 75+ components.
  - `gpui-kit`: one dependency that re-exports everything.
  - `gpui-shell`: optional JS extension runtime.
  - `gpui-webview`: Wry fork `lb-wry`.
  - `gpui-fps`: HUD.
- Components found in `crates/component/src` that matter for Synapse:
  - Chat: **message, bubble, message_scroller (tail-follow virtual list with jump-to-bottom), attachment, avatar**, speech (microphone, recognizer, waveform).
  - Text: **streaming Markdown and HTML** via `TextView`. It reparses incrementally when text is appended, handles a table that is still streaming in, and has `stream_fade`, a per-word fade "the way Claude reveals a reply". Example: `examples/stream-markdown`.
  - Code: **highlighter (tree-sitter, ~30 languages behind features)**, input/editor (200k-line editor with LSP hooks), diff.
  - Layout: **dock (serializable, draggable tabs, nested splits), resizable, tab, sidebar, sheet, dialog, popover, menu, native_menu, command palette, title_bar, status_bar, toolbar**.
  - Forms: **setting (settings pages), form**, select/combobox, switch, slider, date/time, color_picker.
  - Data: table (virtual, 100k+ rows), virtual_list, tree.
  - Feedback: **notification (in-app toasts)**, alert, progress, skeleton, **shimmer**, spinner, badge.
  - Charts: **chart/plot**.
  - Themes: theme and a `themes/` JSON folder.
- Motion (`gpui-base::motion`): transition, **spring**, keyframes, sequence, stagger, presence/reveal, easing, plus a reduce-motion setting. See `docs/STYLING-AND-MOTION.md`.
- AccessKit accessibility, and headless UI integration tests.
- Weak spots:
  - The API churns weekly before 1.0, so expect small migrations on every bump.
  - The `gpui-fast` backend is experimental.
  - Webview needs gpui-fast and system WebView2/WebKitGTK.

## 3. Other kits and libraries (from awesome-gpui)

- Component alternatives:
  - adabraka-ui: 478 stars, MIT, **dormant since 2026-02**.
  - guise: Mantine-like, 135 stars.
  - Bezel: SwiftUI-like, 173 stars.
  - base-gpui: headless, 93 stars.
  - Ely components.
  - GPUIX: React bindings, 2.6k stars. It's heavier, around 143 MB RSS against 57 MB for plain GPUI per its own benchmark PR.
- Helpers:
  - gpui-router, gpui-nav, gpui-hooks, gpui-tea, gpui-form (derive forms for gpui-component).
  - gpui-storybook, gpui-devtools.
  - plotters-gpui, gpui-px/gpui-d3rs (charts), ratex-gpui (LaTeX), gpui-pdf, gpui-video-player, gpui-flow/ferrum-flow (node editors), gpui-symbols (SF Symbols).
  - gpui-book: 274 stars, a learning resource.
- **gpui-tray** (domenkozar, 0.1.4, Apache-2.0). Native tray with AppKit on macOS, Shell_NotifyIconW on Windows, and SNI/DBusMenu via zbus on Linux (no GTK). It runs no second event loop: callbacks go to a task on GPUI's foreground executor, menu items are `gpui::MenuItem`, and clicks dispatch GPUI actions. It has an explicit **`gpui-kit` feature** (gpui-pre 0.3.x). It's small (8 stars) but tailored exactly to this use. Vendor or fork it if needed.

## 4. Animation and the "dots" blob avatar

Upstream GPUI (gpui-pre 0.3.9) has:
- `Animation` with `.with_animation(id, Animation::new(dur).repeat().with_easing(..), |el, t| ..)` (AnimationExt).
- `canvas()` for immediate paint.
- `PathBuilder` for filled and stroked Bezier paths, and `window.paint_path`.
- Linear gradients, shadows with blur, opacity, transforms on SVG, `img()` with GIF (`gif_viewer` example) and SVG rasterized through resvg, plus `svg()` for monochrome icons.

**There are no user-supplied shaders** in upstream or gpui-kit. Drawing the blob:
- Paint it in a `canvas` as a closed cubic Bezier through about 8 points. Displace the radii with a few summed sines driven by time, with an amplitude per state: idle breathing, fast wobble while working, bounce or pulse for "needs you".
- Fill it with a gradient, and draw the eyes as two small rounded quads (or paths) with blink keyframes.
- Use the kit's spring and keyframes for state changes.
- This is cheap: one path per frame per avatar. **Animate only when visible and in an active state.** GPUI only requests frames while an animation runs. An infinite `.repeat()` idle animation keeps the GPU and CPU waking at display refresh rate, so cap the idle "breathing" or pause it when the window is hidden or occluded.

3D or real shaders:
- gpui-ce's wgpu backend and its Discord talk about custom shaders, but nothing is shipped.
- The workable option is gpui-fast **window composition**, which embeds a native surface such as a wgpu or Metal layer, or a WKWebView, under GPUI overlays.
- Another option is offscreen wgpu rendering pushed as an image texture, which is wasteful.
- The cheapest fallback for fancy avatars is pre-rendered sprite sheets or Lottie-like frames. GIF/WebP is supported.

## 5. Desktop integration matrix

| Need | Recommendation | Notes |
|---|---|---|
| Keep running with window closed | **Built in.** `Application::with_quit_mode(QuitMode::Explicit)` or `cx.set_quit_mode`. Default is Explicit on macOS and LastWindowClosed elsewhere | Set Explicit on all OSes and quit only from the tray "Quit". |
| Menu-bar-only / no Dock icon (macOS) | **Built in.** `cx.set_activation_policy(ActivationPolicy::Accessory)`. `App::request_windowing(Headless/Windowed)` also toggles Accessory/Regular. On Linux, headless means no display connection | This is new in recent snapshots. |
| Tray icon + menu | **`gpui-tray`** (feature `gpui-kit`) gets the best integration, with no GTK on Linux. The alternative is **`tray-icon` 0.26.1 + `muda` 0.21** from Tauri | tray-icon must be created on the main thread (inside `Application::run`). On Linux its default backend needs the GTK main loop pumped, which GPUI doesn't do, so use its **KSNI** backend. Forward events with `TrayIconEvent::set_event_handler`: send into an async channel and `cx.spawn` a foreground task that `.await`s it. Never poll with a timer, so idle stays at 0%. |
| Global hotkey | **`global-hotkey` 0.8** (Tauri) | Create the manager on the main thread inside `run`. Use `GlobalHotKeyEvent::set_event_handler` → channel → GPUI task, and filter `HotKeyState::Pressed`. **Wayland is unsupported** (X11 only). On Wayland, use the xdg-desktop-portal GlobalShortcuts via `ashpd` 0.13, or fall back to a desktop-environment keybinding that runs `synapse --toggle`, using single instance plus IPC. |
| OS notifications | **Built in**: `SystemNotification { tag, title, body, actions }` with a response callback (`examples/system_notifications.rs`). Fallback: `notify-rust` 4.18 on Linux/Windows, `mac-notification-sys` on macOS | Unsigned macOS builds may not be allowed to post. Bundle and sign, and keep a stable bundle id. |
| In-app toasts | gpui-kit `notification` | — |
| Keychain | **Built in**: `cx.write_credentials / read_credentials / delete_credentials` (Keychain, Windows Credential Manager, Secret Service). Alternative: `keyring` 4.2 | Built-in is async (Task) and has no extra dependency. |
| Launch at login | `auto-launch` 0.6 (LaunchAgent / registry / XDG autostart) | On macOS 13+, `smappservice-rs` (SMAppService) is the modern, approved path. |
| Single instance | Use a named local socket from `interprocess` 2.4 as both lock and IPC ("show window", "toggle"). `single-instance` is unmaintained (2021) | The IPC listener runs on the background executor and wakes the UI through a channel. |
| Auto-update | `cargo-packager` with `cargo-packager-updater` (signed manifests; .app/.dmg, NSIS/MSI, AppImage/deb). Alternative: `self_update` 1.3 (GitHub releases) | Note that cargo-packager's last release was 0.11.8 in 2025-11, though the repo is active. |
| Packaging and signing | **cargo-packager** (dmg, app, nsis, wix, deb, AppImage, pacman; macOS codesign + notarize; Windows signing hooks). Alternative: cargo-bundle 0.12 | Zed's own `script/bundle-*` are good references. |
| HTTP / tokio libraries (reqwest, rmcp, tokio-tungstenite) | Run your own **tokio runtime with 1–2 worker threads** next to GPUI's executor. Copy Zed's `gpui_tokio` (~100 lines, Apache-2.0; not on crates.io as gpui-pre). `Tokio::spawn(cx, fut)` runs on tokio and returns a GPUI `Task` (dropping the task aborts it) | GPUI's executors (`cx.spawn` foreground, `cx.background_spawn` thread pool) are not tokio. Anything that calls `tokio::spawn` or needs a reactor (reqwest, rmcp, hyper) panics without a runtime context. Keep agent orchestration on tokio. Push UI updates back through `WeakEntity::update` in a `cx.spawn` task, or through a channel. GPUI also ships its own HTTP client abstraction (`reqwest_client` / `gpui-pre-reqwest`) if you only need image loading. |

## 6. Measured footprint and idle cost

Measured numbers:
- Plain GPUI hello world (gpuix PR #603, Sept 2026, one author): **~26 MB physical footprint, 57 MB RSS, 6.6 MB bundle**, in a 400×300 window.
- Idle CPU: an app on a patched gpui-kit fork measured **61 wakeups/s and 0.7% CPU idle on X11** before a patch that stops the frame loop 100 ms after the last frame request, and 0/s and 0% after (request-eagle PR #112). Wayland already parked the loop. The patch is not upstream. Check whether the current gpui-pre still ticks at vsync on X11 and macOS, and measure with the kit's `gpui-fps` HUD (it stops its own clock when hidden).
- gpui-fast retained mode cuts per-frame CPU for streaming text by 73%, which matters for long agent chats.

Expected budget for Synapse:
- RAM: 40–80 MB idle with one window. Fonts, the glyph atlas and tree-sitter grammars dominate. Enable only the tree-sitter languages you need, because every `tree-sitter-*` feature adds MB.
- Closing the window: if you destroy the window, not just hide it, the GPU surface and atlas are freed, so tray mode should drop to roughly 15–30 MB.
- CPU: near 0% when nothing animates.

## 7. Known gotchas

1. **Type identity:** every crate that touches GPUI types (the kit, the tray, your app) must resolve to the same `gpui-pre` version. Mixing git `gpui` with `gpui-pre`, or with `gpui-ce`, gives "expected `gpui::App`, found `gpui::App`" errors.
2. **Weekly breaking changes:** bump `gpui-kit` deliberately (for example monthly), together with the lockfile, and read `release-notes.md`. Don't use `cargo update` blindly.
3. **Infinite animations** keep frames going. Gate avatar animation on visibility and state, and respect reduce-motion.
4. **Linux:**
   - Needs libxkbcommon, wayland, vulkan/mesa and fontconfig.
   - Global hotkeys don't work on Wayland.
   - The tray needs an SNI host (GNOME needs the AppIndicator extension).
   - The webview needs WebKitGTK.
5. **macOS:** call tray, hotkey and menu APIs only on the main thread, inside `Application::run`. Notifications and launch-at-login want a signed .app bundle.
6. **Windows:**
   - Embed the manifest (GPUI's `windows-manifest` feature; gpui-fast uses `windows-manifest-provided-by-host`, so don't double-embed).
   - Set `#![windows_subsystem = "windows"]`.
   - The tray needs the hidden message window that gpui-tray and tray-icon create.
7. **Tokio inside GPUI:** don't `block_on` on the main thread, and don't let a tokio worker hold `Entity` handles. They're `!Send` on the foreground side, so marshal results back.
8. **Release profile:** Zed and the kit set `opt-level=3` for gpui, taffy and tree-sitter even in dev. Copy their `[profile]` blocks. Use LTO or thin LTO, `codegen-units=1` and `strip` in release to shrink the binary.
9. Before 1.0, gpui-kit's own `CLAUDE.md`/`AGENTS.md` and its skills are the most current API docs. Upstream GPUI docs are thin: rustdoc, examples and gpui.rs.

## 8. Concrete starting Cargo.toml

```toml
[dependencies]
gpui-kit = { version = "=0.7.3", default-features = false, features = ["component", "assets", "tree-sitter", "tree-sitter-rust", "tree-sitter-python", "tree-sitter-javascript", "tree-sitter-json", "tree-sitter-bash"] }
# gpui = re-exported as gpui_kit::gpui (gpui-pre =0.3.9)
gpui-tray = { version = "0.1", default-features = false, features = ["gpui-kit", "menu-state"] }
global-hotkey = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "net"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream"] }
interprocess = "2"
auto-launch = "0.6"
keyring = "4"            # optional; GPUI has write_credentials built in
notify-rust = "4"        # optional fallback; GPUI has SystemNotification built in

[features]
fast = ["gpui-kit/gpui-fast"]   # A/B the retained-mode backend
```

Check the exact feature names against `crates/kit/Cargo.toml` before use. Pin `gpui-kit` with `=` and upgrade on purpose.

## Sources

- https://github.com/longbridge/gpui-kit (README, Cargo.toml, docs/STYLING-AND-MOTION.md, crates/*), https://gpui-kit.com, https://gpui-kit.com/docs/fps/
- https://github.com/longbridge/gpui-fast
- https://crates.io/crates/gpui-pre, https://crates.io/crates/gpui, https://crates.io/crates/gpui-ce
- https://github.com/gpui-ce/gpui-ce, https://gpui-ce.github.io/, https://github.com/gpui-ce/gpui-ce/issues/278
- https://github.com/zed-industries/zed (crates/gpui README, crates/gpui_tokio), https://github.com/zed-industries/zed/discussions/40318
- https://github.com/zed-industries/awesome-gpui, https://github.com/zed-industries/create-gpui-app
- https://github.com/domenkozar/gpui-tray, https://docs.rs/adabraka-gpui
- https://docs.rs/tray-icon, https://docs.rs/global-hotkey
- https://gpuix.dev/, https://github.com/remorses/gpuix
- https://github.com/Ernxst/gpuix/pull/603, https://github.com/gregor-tokarev/request-eagle/pull/112
- https://github.com/eidola-ai/eidola/pull/373, https://github.com/usadamasa/twigpui/issues/284
