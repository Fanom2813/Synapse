# Synapse: milestones

Build order for the unattended loop (sg). Each milestone has a goal, an **accept** command (the
milestone's acceptance test file in `crates/synapse-acceptance/tests/`), and feature-sized tasks with a
**verify** command. Acceptance and verify tests run offline with mocks (`MockLlm`, `MockSearch`,
`wiremock`, GPUI `TestAppContext`); nothing needs a real API key. Live checks are behind `SYNAPSE_LIVE=1`.

Full project check (also in `.sg/config.toml`):
`cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

Conventions for tests:
- Acceptance file per milestone: `crates/synapse-acceptance/tests/mNN_<name>.rs`; it drives the real
  crates (and GPUI views via `TestAppContext`) through the user flows listed under the milestone.
- UI tests open views, click/type through actions, and assert on entity state and rendered text.
- Task verify commands use `cargo test -p <crate> <filter>`; the builder writes those tests.

---

## M1 — Foundation
Goal: an app window that opens on macOS/Windows/Linux, with the workspace, store, tokio bridge, theme,
tray and hide-to-tray in place, and idle CPU near zero.
accept: `cargo test -p synapse-acceptance --test m01_foundation`

1. **Cargo workspace + CI.** Crates from ARCHITECTURE.md §2 (empty libs compile), pinned
   `[workspace.dependencies]` from STACK.md, release profile from PERFORMANCE.md, `rust-toolchain.toml`,
   GitHub Actions matrix (macOS, Windows, Ubuntu) running the full check. verify: `cargo build --workspace && cargo test --workspace`
2. **App shell window.** `synapse` binary opens a GPUI Kit window with sidebar / main / right-panel dock
   layout (empty states), custom title bar per UI.md §1b (native window controls per OS, drag/zoom, sidebar and
   panel toggles, command palette button; other slots as placeholders), motion tokens module (UI.md §5), theme (system/light/dark) switch persisted; fonts (UI.md §6, subset script) and Phosphor icons
   (UI.md §7, sync script, GPUI Kit icon override) wired in. verify: `cargo test -p synapse-app shell`
3. **Tokio bridge.** Copy Zed's gpui_tokio pattern (license header kept): 2-thread runtime, spawn returning
   GPUI tasks, cancellation on drop; a demo command fetches a URL via reqwest without blocking UI. verify: `cargo test -p synapse-app tokio_bridge`
4. **Store.** `synapse-store` with rusqlite bundled, WAL, migrations for the full schema in
   ARCHITECTURE.md §3, single writer task, read pool, FTS5 tables; repo fns for projects/agents/messages. verify: `cargo test -p synapse-store`
5. **Settings + data dirs + logging + secret redactor.** Settings table API, data/log dirs via `dirs`, tracing with
   rolling file; the secret redactor (ARCHITECTURE.md §6: known values + gitleaks rules + default deny paths),
   on by default, wired into logging now and into every model-bound path as those paths are built. verify: `cargo test -p synapse-app settings && cargo test -p synapse-core redact`
6. **Tray, hide-on-close, single instance.** `QuitMode::Explicit`; close hides window, window destroyed after
   60 s hidden and rebuilt on open; gpui-tray menu (Open, Quit); `interprocess` single instance (second
   launch focuses the first). verify: `cargo test -p synapse-app lifecycle`
7. **Perf probe + size/perf CI gates.** `--perf-probe idle|tray --seconds N` printing JSON (CPU, RSS, footprint);
   a test that the idle window does not request frames when nothing animates; CI job (macOS) that builds
   release, checks binary size, `cargo bloat --crates` report, `cargo tree -d` duplicate check, and runs the
   idle/tray probes against the hard caps in PERFORMANCE.md §1. verify: `cargo test -p synapse-app perf_probe`

## M2 — Providers and a single agent chat
Goal: the user adds any provider from the models.dev catalog (or a local server), stores the key in the
keychain, picks a model, and chats with one agent with streaming markdown.
accept: `cargo test -p synapse-acceptance --test m02_providers_chat`

1. **Catalog.** Embedded gz snapshot of models.dev `api.json` (trimmed to fields we use), daily refresh with
   ETag into `catalog_cache`, typed lookup (providers, models, limits, cost, capabilities). verify: `cargo test -p synapse-llm catalog`
2. **LlmClient over genai.** `ModelBackend` impl for API models: streaming text, tool calls, usage, errors
   mapped; custom OpenAI-compatible base URL; `MockLlm` scripted backend for tests. verify: `cargo test -p synapse-llm backend`
3. **Secret store + local detection.** `SecretStore` per ARCHITECTURE.md §6 (OS keychain, encrypted-file
   fallback on Linux without Secret Service, `SecretString`, per-host sending, masked UI, delete on remove,
   redaction); tests prove no secret value reaches the DB, logs, exports or another host. Detect Ollama
   (:11434) and LM Studio (:1234) and list their models; "test connection". verify: `cargo test -p synapse-llm keys_local`
4. **Providers settings screen.** Searchable catalog list, enable provider, enter key, test, set default
   models per role (lead, member, selector). verify: `cargo test -p synapse-app settings_providers`
5. **Chat view.** Message list (GPUI Kit scroller, virtualised), streaming markdown with word fade-in,
   code highlighting, composer with send⇄stop morph and kept draft, coalesced deltas per frame. verify: `cargo test -p synapse-app chat_view`
6. **Cost tracking.** Usage × catalog price stored per message; totals per conversation shown in header. verify: `cargo test -p synapse-llm cost`

## M3 — Agents, personas and avatars
Goal: the user creates and customises agents (backend, instructions, look), imports personas from GitHub
collections, and assembles a team from templates.
accept: `cargo test -p synapse-acceptance --test m03_agents`

1. **Agent model + CRUD.** Create/edit/duplicate/delete; backend (api model), params, instructions, tools list. verify: `cargo test -p synapse-core agents`
2. **Agent editor sheet.** All fields from UI.md §2.3 with validation. verify: `cargo test -p synapse-app agent_editor`
3. **Avatar renderer.** Canvas blob with shapes, palettes, eyes, mouth, accessories (UI.md §4), sizes,
   deterministic default from name. verify: `cargo test -p synapse-app avatar_render`
4a. **Avatar packs.** `scripts/gen-avatars` pre-renders DiceBear CC0/free styles + Fluent role badges into
   `assets/avatars/` with a license manifest and CREDITS.md (UI.md §4); picker tab in the avatar editor. verify: `cargo test -p synapse-app avatar_packs`
4. **Avatar states + editor.** State animations (idle/thinking/speaking/working/needs-you/paused/error),
   settle after 30 s, stop when hidden or reduce-motion; editor with live preview and "surprise me". verify: `cargo test -p synapse-app avatar_states`
5. **Persona parsing.** Markdown + YAML frontmatter (agency-agents, wshobson, VoltAgent, 0xfurai, lst97 formats),
   Fabric `system.md` patterns, SKILL.md skills (anthropics/skills, K-Dense) into agent/skill drafts; repo, sha, license recorded;
   curated offline starter set in `assets/personas/`. verify: `cargo test -p synapse-personas parse`
6. **Persona library.** Sources from PRD.md §5.2 fetched as GitHub tarballs (cached), search, category filter,
   preview, import one/many; license badge, unlicensed sources link-only; import from URL or local file. verify: `cargo test -p synapse-app persona_library`
6b. **Skills runtime + library.** Skill folders (agentskills.io format) at global/project/agent scope;
   `load_skill` / `read_skill_file` tools with progressive disclosure; scripts only via the shell tool and
   permissions; Skill library screen (search, preview, enable per agent, update, delete); import from GitHub
   collections, URL/zip/folder and existing local skill folders (`~/.claude/skills`, `~/.codex/skills`,
   `.agents/skills`, opt-in); `/skill-name` slash menu in the composer; built-in starter skills. verify: `cargo test -p synapse-core skills && cargo test -p synapse-app skill_library`
7. **Projects + team templates.** Create project (name, goal, folder), choose template (Research squad,
   Product brainstorm, Investment analysis, Custom), lead agent, add/remove members. verify: `cargo test -p synapse-core projects`

## M4 — Team room: agents talking in real time
Goal: a team run where agents ask, answer, challenge, refuse, delegate and converge, streamed live, with
the user able to interject, pause and stop.
accept: `cargo test -p synapse-acceptance --test m04_team_room`

1. **Bus + envelopes.** Types from TEAM-PROTOCOL.md §2–3, persist-then-publish, lag resync. verify: `cargo test -p synapse-core bus`
2. **Team tools.** say/ask_agent/answer/challenge/agree/refuse/delegate/report_finding/propose/vote/ask_user/done
   as tool definitions with schemars schemas → envelopes. verify: `cargo test -p synapse-core team_tools`
3. **Agent worker + context builder.** Turn loop per agent, context per TEAM-PROTOCOL.md §7, rolling summary,
   safe-point inbox checks, steering and interrupts (§3b), prompt layout for caching (§3c). verify: `cargo test -p synapse-core worker`
4. **Orchestrator + run state machine.** Phases and mode presets (§4), speaker selection (§5), limits and
   budgets (§6), convergence, pause/resume/stop, interrupted-run resume. verify: `cargo test -p synapse-core orchestrator`
4b. **Title bar live status.** Project switcher, run pill with live avatars and phase/progress line, run
   popover controls, approvals inbox with inline approve/deny, budget meter, overflow on narrow windows (UI.md §1b). verify: `cargo test -p synapse-app title_bar`
5. **Team room view.** Typed message styles (UI.md §3), threads, live avatars and states in header, three-layer
   detail toggle, @mentions and interjection, run controls, budget meter. verify: `cargo test -p synapse-app team_room`
5b. **Attachments.** Content store with hash de-dup, `attach` / `read_attachment` tools, per-recipient
   delivery (vision vs caption, inline vs summary, ACP blocks), thumbnails/viewer/file chips, drag-and-drop
   in the composer, limits (TEAM-PROTOCOL.md §3a). verify: `cargo test -p synapse-core attachments && cargo test -p synapse-app attachments_ui`
6. **Conversation map.** Graph panel of agents and Ask/Delegate/Challenge edges, click edge → jump to message. verify: `cargo test -p synapse-app conversation_map`
7. **Synthesis artifact.** Writer produces the artifact (versions, changelog, dissent, open questions); Artifact panel with diff between versions;
   markdown mirror in the project folder (`reports/`, history, optional transcripts) with atomic writes and edit-import (ARCHITECTURE.md §3). verify: `cargo test -p synapse-core artifacts && cargo test -p synapse-app artifact_panel`

## M5 — Deep research
Goal: Deep research mode searches, reads and follows leads, returns cited findings, critiques and revises
into a sourced report.
accept: `cargo test -p synapse-acceptance --test m05_deep_research`

1. **Search providers.** Trait; keyless `auto` chain ported from ketch (parallel → exa → keenable →
   youcom → firecrawl → ddg) so research works with no keys; `multi` (RRF fusion) and `random` modes;
   SearXNG/Degoog; keyed APIs ported from search-cli; provider-native search; external `ketch`/`search`
   binaries; rate limits + circuit breaker; Settings section with backend health check. verify: `cargo test -p synapse-research search`
2. **Reader + cache.** Fetch, robots, rate limits, readability → markdown, PDF, Jina fallback, source cache,
   chunking. verify: `cargo test -p synapse-research reader`
3. **Research loop.** Breadth/depth loop (DEEP-RESEARCH.md §3), budget forcing, compressed findings with citations. verify: `cargo test -p synapse-research loop`
4. **Perspectives + outline.** STORM-style perspectives mapped to members, Q&A with grounded expert, outline. verify: `cargo test -p synapse-research perspectives`
5. **Critique, revise, fact-check.** Challenges linked to paragraphs, revision with changelog, quote check against sources. verify: `cargo test -p synapse-research factcheck`
5b. **Agent Builder.** Create an agent from a prompt (PRD.md §5.2): clarify questions, deep research,
   draft persona + knowledge pack with citations + avatar + skills/model suggestion, review with test chat,
   save/export; culture/people guardrails; "refresh knowledge"; `propose_agent` tool for agents mid-run with
   approval card; "build a team for this goal". Tests use MockLlm + MockSearch. verify: `cargo test -p synapse-core agent_builder && cargo test -p synapse-app agent_builder_ui`
6a. **Browser bridge (user's browser).** `BrowserBridge` trait; backend A (chromiumoxide autoConnect to the
   user's Chrome) and backend B (chrome-devtools-mcp / Playwright MCP Bridge as connectors); `browser:*`
   search providers and `browser_*` tools; Synapse window/tab group only, human-pace limits, CAPTCHA/login
   hand-off (notify user, auto-resume when cleared, fall back after 10 min), domain block list, form actions always ask; Settings → Browser setup
   guide. Tests use a fake CDP endpoint. verify: `cargo test -p synapse-research browser`
6. **Sources panel.** List, dedupe, which agent used each, preview, open in browser, citation hover popovers. verify: `cargo test -p synapse-app sources_panel`

## M6 — Tools, permissions and MCP
Goal: agents use file, shell and MCP tools under allow/ask/deny rules with approval cards and an audit log.
accept: `cargo test -p synapse-acceptance --test m06_tools_mcp`

1. **Permission engine.** Rules by scope (global/project/agent), deny > ask > allow, globset patterns, modes; natural-language rule review (optional cheap model). verify: `cargo test -p synapse-tools permissions`
2. **File tools.** read/list/write/edit inside project folder only (canonicalise, symlink escape rejected), diff preview. verify: `cargo test -p synapse-tools fs_tools`
3. **Shell tool.** Off by default; nono sandbox (macOS/Linux), timeout, output cap; Windows always asks. verify: `cargo test -p synapse-tools shell_tool`
4. **Approval cards + audit log.** Inline card (approve once / always / deny) and OS notification when hidden;
   audit view with redacted args. verify: `cargo test -p synapse-app approvals`
5. **MCP client manager.** rmcp stdio + streamable HTTP + OAuth, lazy start, idle kill, tools exposed to
   agents with per-agent/per-tool toggles, read-only annotation auto-run. verify: `cargo test -p synapse-mcp client`
6. **MCP settings + registry browser.** Add/edit servers, browse registry.modelcontextprotocol.io, install config, test. verify: `cargo test -p synapse-app mcp_settings`
7. **Connectors gallery.** Curated `assets/connectors.json` (remote OAuth connectors, local connectors,
   aggregators per PRD.md §5.5; each URL verified against the registry/vendor docs, unverifiable ones
   dropped), one-click connect with OAuth, connector cards with read/write tools and per-agent access;
   onboarding "connect apps" step uses it. Tests use a mock MCP server with a fake OAuth flow. verify: `cargo test -p synapse-app connectors`
8. **Synapse as MCP server.** Opt-in rmcp server (stdio + localhost HTTP with token) exposing list_projects,
   start_run, run_status, get_report, ask_agent; Settings shows config snippets for Claude Desktop, Cursor, Zed. verify: `cargo test -p synapse-mcp server`

## M7 — Installed CLI agents (Claude Code, Codex, OpenCode, Gemini, ...)
Goal: agents can be backed by CLIs the user already has, driven over ACP, taking part in the team with the same tools and approvals.
accept: `cargo test -p synapse-acceptance --test m07_cli_agents`

1. **Detection.** Login-shell PATH resolution, ACP registry fetch (cached), detect installed CLIs and adapters, login state where knowable. verify: `cargo test -p synapse-acp detect`
2. **ACP client backend.** Spawn, initialize, session/new, prompt, stream updates, cancel, map to `ModelBackend` events; usage when provided. Tests use a fake ACP agent binary built in the test. verify: `cargo test -p synapse-acp client`
3. **Permissions bridge.** ACP `session/request_permission` → approval cards; file/terminal capabilities limited to the project folder. verify: `cargo test -p synapse-acp permissions`
4. **Team tools for CLI agents.** In-process MCP server exposing team tools so CLI agents can ask/delegate/report. verify: `cargo test -p synapse-acp team_mcp`
5. **Headless fallbacks.** `claude -p` stream-json, `codex exec --json`, `opencode run --format json`, `gemini -p` stream-json parsers for CLIs without ACP. verify: `cargo test -p synapse-acp headless`
6. **CLI agents settings + picker.** Settings section and backend picker entries ("use Claude Code as agent"). verify: `cargo test -p synapse-app cli_agents_ui`

## M8 — Memory and search
Goal: agents remember what matters across runs and projects (MEMORY.md), the user can see and control
every memory, and everything is searchable.
accept: `cargo test -p synapse-acceptance --test m08_memory`

1. **Memory store + scopes.** `memories` table, scopes (user/project/agent/digest), supersede via
   `invalid_at`, markdown mirror files with edit-back parsing. verify: `cargo test -p synapse-store memories && cargo test -p synapse-core memory_files`
2. **Semantic index.** model2vec-rs lazy load, embeddings into sqlite-vec, hybrid FTS5 + vector retrieval. verify: `cargo test -p synapse-store vector`
3. **Writing memory.** `remember` tool, "Remember" action on messages, run digests, automatic extraction with
   mem0-style ADD/UPDATE/DELETE/NOOP decisions, proposals for project/user scope, secret filter. verify: `cargo test -p synapse-core memory_write`
4. **Recall in context.** Core block + retrieved block within the memory budget, `recall` / `search_archive`
   tools, "used N memories" on messages. verify: `cargo test -p synapse-core memory_recall`
5. **Memory screens.** Settings → Memory and Project → Memory: list, search, edit, pin, delete, provenance,
   history, review queue, forget run/scope, pause per project. verify: `cargo test -p synapse-app memory_screens`
6. **Skill creator + learned skills.** Built-in `skill-creator` skill and editor with preview; after a run the
   lead may propose "save as skill"; user reviews/approves; saved as a normal skill folder. verify: `cargo test -p synapse-core learned_skills`
7. **Global search.** Command-palette search over messages, sources, artifacts, memories with jump-to. verify: `cargo test -p synapse-app global_search`

## M9 — Background, notifications, schedules, quick ask
Goal: Synapse works from the tray: runs continue with the window closed, notifications bring the user back,
schedules run while the app is running, and a hotkey opens quick ask.
accept: `cargo test -p synapse-acceptance --test m09_background`

1. **Tray status menu.** Active runs with phase, pending approvals count, pause all, quick ask, quit; tray icon state (idle/working/needs-you). verify: `cargo test -p synapse-app tray_menu`
2. **Notifications.** Approval needed, run finished, agent asks user, error; click opens the right place; toasts when focused. verify: `cargo test -p synapse-app notifications`
3. **Global hotkey + quick-ask window.** Configurable combo (Wayland via portal), compact streaming answer, open in app. verify: `cargo test -p synapse-app quick_ask`
4. **Schedules.** Natural-language → cron spec (confirmed by user), runner sleeps until due, missed runs listed with run-now, Scheduled tab. verify: `cargo test -p synapse-core schedules`
5. **Launch at login + background settings.** auto-launch toggle, keep running on close toggle, menu-bar-only mode (macOS). verify: `cargo test -p synapse-app background_settings`
6. **Activity tab.** Per agent/project: runs, tool calls, approvals, with pause/resume/cancel/retry. verify: `cargo test -p synapse-app activity_tab`

## M10 — Onboarding, export, packaging and performance
Goal: a first-time user is onboarded in under 2 minutes, results export, installers build for all
platforms, and the performance budgets hold.
accept: `cargo test -p synapse-acceptance --test m10_release`

1. **Onboarding flow.** UI.md §9: seven steps, background detection of CLIs/local models/keys, live key test,
   "try free" path, team template, first project with the onboarding-to-main-window morph and lead introduction;
   skippable, resumable, keyboard-first; acceptance asserts no step waits on network. verify: `cargo test -p synapse-app onboarding`
2. **Export.** Artifacts and transcripts to Markdown/HTML, PDF via print; open project folder. verify: `cargo test -p synapse-core export`
3. **Packaging + updates.** cargo-packager configs (dmg/app, msi/nsis, AppImage/deb), icons, updater against GitHub Releases, release workflow on tag. verify: `cargo packager --help >/dev/null && cargo test -p synapse-app updater`
3b. **Motion polish pass.** Walk the micro-interaction list in UI.md §5 and make every item match the motion
   tokens (one consistent feel), add any missing ones, verify reduce-motion and that idle returns to zero frames. verify: `cargo test -p synapse-app motion`
4. **Performance pass.** `--perf-probe run5|scroll`, compare `fast` feature, fix regressions until PERFORMANCE.md §1 holds; record results in `docs/PERF-RESULTS.md`. verify: `cargo test -p synapse-acceptance --release --test m10_performance -- --include-ignored`
5. **User docs.** README quick start, screenshots, provider/CLI/MCP how-tos in `docs/user/`. verify: `test -f docs/user/getting-started.md`

---

## Later (label `later`, built only with `sg run --quality`)
Voice calls (realtime speech, captions, minimise to chat); own browser extension + native messaging (backend C);
in-app live view of the agent's browser tab; chat channels while running (Telegram first); teach-by-demonstration routines;
Ideas surface (proactive suggestions while open); pages/spaces editor with slash commands; plugin/skill
marketplace; i18n; accessibility audit.
