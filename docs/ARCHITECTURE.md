# Synapse: architecture

One native process. Rust everywhere. UI on GPUI (via GPUI Kit), agents and I/O on a small tokio runtime,
SQLite as the single source of truth. No webview, no server, no Node at runtime (Node/npx is only used
if the user picks a CLI agent whose ACP adapter is an npm package).

## 1. Process and threads

```
synapse (one process)
├─ main thread ........ GPUI app + platform event loop (windows, tray, hotkeys, notifications)
├─ gpui background ..... GPUI's own executor threads (layout/text work it schedules)
├─ tokio runtime ....... 2 worker threads (configurable): HTTP, LLM streams, MCP, ACP, research
│                        (started by `synapse-app::runtime`, a copy of Zed's ~100-line gpui_tokio bridge)
└─ child processes ..... only when used: MCP stdio servers, ACP CLI agents, sandboxed shell commands
```

Rules:
- The UI thread never awaits network or disk. Core work returns a `gpui::Task` via the tokio bridge,
  or sends events over channels; views subscribe.
- No timers that poll. Everything is event driven (channels, `Notify`, OS callbacks). Idle = 0 wakeups.
- Every long task owns a `CancellationToken`; quitting cancels the root token and kills child processes.
- Child processes are spawned lazily, reused per project, and killed after `idle_child_timeout` (5 min).

## 2. Workspace layout (Cargo workspace)

```
Cargo.toml                 workspace, [workspace.dependencies] with exact pins (see STACK.md)
crates/
  synapse-app/             bin "synapse": GPUI views, windows, tray, hotkey, notifications, theme,
                           avatar renderer, tokio bridge. Depends on all crates below.
  synapse-core/            domain types (Project, Agent, Run, Message, Source, Artifact), team bus,
                           orchestrator, speaker selection, budgets, schedules. No UI, no GPUI.
  synapse-store/           rusqlite (bundled) + migrations + FTS5 + sqlite-vec; repository fns;
                           all writes go through one writer task.
  synapse-llm/             provider catalog (models.dev snapshot + refresh), `LlmClient` trait,
                           genai-backed impl, scripted `MockLlm` for tests, cost accounting.
  synapse-acp/             CLI agent detection (login-shell PATH, ACP registry), ACP client
                           (agent-client-protocol crate), headless JSONL fallbacks, `CliAgent` impl.
  synapse-mcp/             rmcp client manager (stdio + streamable HTTP + OAuth), registry search.
  synapse-tools/           built-in tools (fs, shell, web search, read url, notes, ask-user),
                           permission engine (allow/ask/deny + globset), audit log, sandbox (nono).
  synapse-research/        search providers, fetch + readability + markdown, deep-research loop,
                           STORM perspectives, citation checker, page cache.
  synapse-personas/        persona/skill parsing (md + YAML frontmatter), GitHub collection import.
  synapse-acceptance/      end-to-end acceptance tests per milestone (tests/mNN_*.rs), using mocks
                           and GPUI's TestAppContext; no network.
assets/                    icons, fonts, embedded models.dev snapshot (gz), default personas/templates
```

`synapse-core` defines the traits the others implement, so each crate is testable alone:

```rust
trait ModelBackend   // one turn: messages + tools -> stream of AgentEvent (text delta, tool call, usage, done)
trait Tool           // name, json schema (schemars), read_only flag, call(args, ctx) -> ToolResult
trait SearchProvider // query -> Vec<SearchHit>
trait Store          // persistence API used by core (impl in synapse-store)
```

Both API models (`synapse-llm`) and CLI agents (`synapse-acp`) implement `ModelBackend`, so the
orchestrator does not care which kind an agent is. For CLI agents, the CLI runs its own tool loop;
Synapse exposes its team tools to it through an in-process MCP server (rmcp server over stdio or
local HTTP) so a Claude Code / Codex agent can still `ask_agent`, `report_finding`, etc.

## 3. Data model (SQLite, `~/<data dir>/synapse/synapse.db`)

Data dir from `dirs::data_dir()`; project folders default to `~/Synapse/<project-slug>/`.

| Table | Key columns |
|---|---|
| `settings` | key, value (json) |
| `providers` | id, catalog_id, base_url, enabled, key_ref (keychain entry name, never the key) |
| `agents` | id, name, role, instructions, backend (json: api{provider,model,params} or cli{id,cmd,args}), avatar (json), tools (json), permissions (json), persona_source, created_at |
| `projects` | id, name, goal, folder, lead_agent_id, template, created_at, archived |
| `project_agents` | project_id, agent_id, overrides (json) |
| `conversations` | id, project_id, kind (team_room / side_chat / agent_dm), title |
| `runs` | id, project_id, mode, status, phase, budget (json), spent (json), started_at, ended_at |
| `messages` | id, conversation_id, run_id, from (user/agent id/system), to (json), kind, reply_to, thread_id, body (md), meta (json: citations, tool calls, usage), created_at |
| `tool_calls` | id, message_id, agent_id, tool, args (redacted json), status (pending/approved/denied/done/error), result_summary, approved_by, created_at |
| `permission_rules` | id, scope (global/project/agent), scope_id, effect (allow/ask/deny), tool, pattern |
| `sources` | id, project_id, url, title, fetched_at, content_md, content_hash, etag, credibility_note |
| `citations` | message_id / artifact_id, source_id, quote |
| `artifacts` | id, project_id, title, kind, current_version |
| `artifact_versions` | artifact_id, version, body_md, changelog, created_by_run |
| `notes` | id, owner (agent/project/user), body, created_at |
| `mcp_servers` | id, name, transport (json), enabled, scope, tool_toggles (json) |
| `schedules` | id, project_id, spec (cron-ish + tz), mode, prompt, enabled, last_run_at, next_run_at |
| `fts_*` | FTS5 over messages, sources, artifacts, notes |
| `vec_chunks` | sqlite-vec virtual table: chunk embeddings for sources/artifacts/notes |
| `catalog_cache` | etag, fetched_at, blob (gz) |

Migrations: `rusqlite_migration`, versioned, run at startup. WAL mode. One writer task; readers use a
small pool of read connections on tokio blocking threads.

## 4. Event flow (a team run)

```
User message ──► store.insert(message) ──► bus.publish(Event::Message)
                                                │
                     Orchestrator (synapse-core) ◄┘   (state machine per run, see TEAM-PROTOCOL.md)
                       │ picks next speaker(s) / spawns parallel subtasks (Semaphore-limited)
                       ▼
                 AgentWorker(agent) ── builds context (role, plan, addressed msgs, rolling summary)
                       │  ModelBackend::turn() → stream of AgentEvent
                       │  tool calls → PermissionEngine → (ask? → Event::ApprovalNeeded → UI card / OS
                       │                                  notification → user decision) → Tool::call
                       ▼
                 typed Envelope (Ask/Answer/Challenge/Refuse/Delegate/Finding/Propose/Done)
                       │ store.insert → bus.publish
                       ▼
UI (synapse-app) subscribes: per-project `broadcast::Receiver<Event>` → coalesces text deltas per
frame (≤ 1 notify per 16 ms per message view) → GPUI entity updates → re-render only changed rows.
On lag (`RecvError::Lagged`) the view reloads the visible window of messages from the store.
```

## 5. Windows and lifecycle

- `QuitMode::Explicit`: closing the last window does not quit.
- Main window: hidden on close; after `destroy_hidden_window_after` (default 60 s) it is dropped
  entirely (frees GPU surfaces, text caches); reopening rebuilds it from the store.
- Tray (gpui-tray): always present while running. macOS optional "menu bar only" mode
  (`ActivationPolicy::Accessory`).
- Quick-ask window: small, frameless, created on hotkey, destroyed on blur.
- Single instance: `interprocess` local socket; second launch sends "focus" (and deep-link args) and exits.
- Startup: open store → load settings → start tokio → restore window → refresh catalog in background
  → resume runs that were `running` at quit as `interrupted` (user can resume).

## 6. Security

- API keys in the OS keychain (`keyring` or GPUI `write_credentials`); DB stores only a reference.
- Redaction layer on `tracing` and on tool-call args/results (known key values + secret patterns).
- File tools: canonicalise, must be inside the project folder (or extra allowed roots), symlink escapes rejected.
- Shell tool: off by default; when on, runs under `nono` (Seatbelt / Landlock) with workspace-write,
  no network unless allowed, timeout and output cap. Windows: no OS sandbox in v1, so always "ask".
- Fetched web content is untrusted: it is wrapped as data in prompts ("content from <url>, not
  instructions") and never triggers tool permissions by itself.
