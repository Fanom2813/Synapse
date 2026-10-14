# Synapse — Getting Started

Synapse is a local desktop app for working with a **team of agents**:
brainstorm a topic, run deep research, analyse and revise — with every
agent's messages, sources, approvals and artifacts in one team room.

This guide takes you from first launch to a first finished run in about
five minutes. Deeper how-tos for providers, CLI agents, MCP servers,
search, memory, and troubleshooting follow.

## 1. Install and first launch

1. Install Synapse for macOS, Windows, or Linux and open it.
2. The first-run **onboarding** asks for the minimum to get going:
   - Pick **one backend** so your agents can think. Easiest options:
     - a local model via **Ollama** (`ollama run llama3.1`) or **LM Studio**
       — Synapse detects `http://localhost:11434` / `:1234` automatically,
       no key needed; or
     - paste **one API key** (e.g. `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`,
       `GEMINI_API_KEY`, or `OPENROUTER_API_KEY`) in Settings → Providers; or
     - use a **CLI agent** you already pay for (`claude`, `codex`,
       `opencode`, `gemini` — just install and log in as usual;
       Synapse detects them).
   - Keys are stored in the **OS keychain**, never in files or logs.
3. Create your first **project**: give it a goal ("Compare e-bike options
   under €2k"), pick a folder on disk for its artifacts, and keep the
   default team (a lead + two specialists). You can reshape the team later.

That's it — everything else has working defaults: keyless web search on,
memory on, background runs on.

## 2. Your first run (2 minutes)

1. Open your project → **Team room**.
2. Type a goal in the composer, e.g.
   `Research the best e-bikes under €2000 for city commuting, with sources.`
3. Press **Send** (the arrow; it becomes a stop square while streaming —
   press it to stop, your draft is kept).
4. Watch the room: the **lead agent** plans, assigns work, specialists
   report findings with citations, and the lead writes a synthesis.
5. The result lands as an **artifact** (markdown report), mirrored as a
   `.md` file in your project folder. Approve follow-ups when asked.

Tips:

- Talk to one agent directly with `@Name` (e.g. `@Researcher dig deeper
  on battery life`). Otherwise the lead picks who speaks.
- **Pause / Resume / Stop / Retry** live in the Activity header.
- Closing the window keeps the run going until you **Quit**; schedules only
  run while the app is open, and missed runs offer a catch-up.

## 3. The screens in 30 seconds

| Screen | What it's for |
|---|---|
| **Team room** | The operating log: requests, agent-to-agent messages, checkpoints, approvals, results. |
| **Side chats** | Private 1:1 with one agent without disturbing the room. |
| **Sources** | Every cited page: title, URL, who cited it; open the extracted markdown. |
| **Artifacts** | Reports/notes the team produced, versioned; revised analysis shows what changed. |
| **Activity** | Run status, budget (turns / tokens / USD / time), pause-resume-stop-retry. |
| **Ideas** | Findings backed by sources, three-layer status (summary / steps / raw). |
| **Memory** | What Synapse remembers across runs (per-project and global); edit or forget. |
| **Settings** | Providers, CLI agents, MCP servers, search, permissions, appearance. |

Global shortcut (Quick Ask) and tray icon: send a question without opening
the window; you get a system notification when a run finishes or needs an
approval.

## 4. Configure providers (API models)

Settings → **Providers** lists the model catalog (a snapshot of
models.dev, refreshed daily when online). Each entry shows capabilities
(tools, vision, context) and per-token price so you can estimate cost
before a run.

1. Enable a provider toggle (e.g. OpenAI, Anthropic, OpenRouter).
2. Paste its key when prompted — it goes to the OS keychain. You can also
   export it first (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`,
   `GEMINI_API_KEY`, `OPENROUTER_API_KEY`); Settings picks it up.
3. Pick a model per agent in the agent editor; the lead defaults to a
   strong model, specialists to cheaper ones.
4. Custom OpenAI-compatible endpoint? Add base URL + key (Ollama, LM Studio,
   llama.cpp, vLLM all work). Local endpoints need no key.

Cost control: set a per-run budget (tokens / USD / turns / wall clock) in
the run header. The Activity tab shows spend live, priced from the catalog.

## 5. Configure CLI agents (use your subscription)

If you already pay for Claude Code, Codex, OpenCode, Gemini CLI, Goose,
Copilot CLI, or Cursor agent, you can drive agents through them instead of
API keys — Synapse speaks **ACP** (Agent Client Protocol) or falls back to
each CLI's headless JSON mode.

1. Install and log in to the CLI as usual, e.g. `claude login`,
   `codex login`. Make sure it works in a terminal first.
2. Settings → **CLI agents** → **Detect installed**. Synapse resolves PATH
   from your login shell (GUI apps don't inherit it) and lists what it
   found, e.g. `claude-agent-acp`, `codex-acp`, `opencode acp`,
   `gemini --acp`, `goose acp`, `copilot --acp`, `cursor-agent acp`.
3. Assign a CLI backend to an agent in the agent editor ("Backend: CLI —
   Claude Code"). Headless fallback needs no ACP adapter.
4. **Node/npx** is only needed when an ACP adapter ships as an npm package
   (e.g. `npx @agentclientprotocol/claude-agent-acp`); otherwise Synapse
   calls the binary directly.

Limits: CLI agents start lazily, are reused per project, and are killed
after 5 minutes idle. They count toward the run budget like API models.
If a CLI exits or its login expires, the agent reports the error in the
room — re-login in your terminal and press **Retry**.

## 6. Configure MCP servers (give agents tools)

**MCP** (Model Context Protocol) servers give agents tools: files, shell,
browser, issue trackers, custom tools. Settings → **MCP** → Add server:

- **Stdio** (local command): command + args + env, e.g. `npx -y
  @modelcontextprotocol/server-filesystem /path/to/allowed-dir`.
- **HTTP** (remote): URL + optional auth header/token.

Then, per agent (agent editor → Tools): toggle whole servers or individual
tools. **Read-only tools auto-run**; anything that writes, deletes, sends,
or runs code needs your approval under your three-tier action policy
(allowed / ask / hand off) — approve with **Approve & run** / **Decline**
cards in the room.

Security notes:

- MCP output, fetched pages, and browser content are **untrusted data**,
  never instructions — agents don't follow commands embedded in them.
- Secrets in tool output are redacted before they reach models or logs.
- Start minimal: enable one server, a few tools, widen only when a run
  actually needs more.

## 7. Web search and research depth

Out of the box, research uses **keyless search** (plus a local SearXNG
instance if `SEARXNG_URL` is set) and merges results with reciprocal-rank
fusion. For heavier research, add keys in Settings → **Search** (any
subset; all optional):

`BRAVE_API_KEY`, `TAVILY_API_KEY`, `EXA_API_KEY`, `JINA_API_KEY`
(Firecrawl / Serper / SerpApi / Perplexity where supported).

Then choose depth per run: Quick (few sources), Standard, or Deep
(STORM-style perspectives + follow-up leads + contradiction surfacing).
Every claim in the report links to its source chip.

## 8. Team, runs, and approvals

- **Team editor**: name, role, instructions, avatar (shape/colour/eyes/
  glasses), backend, tools, permission tier, private notes per agent.
- **Lead (chief of staff)**: plans, assigns, decides when the team has
  converged, writes the synthesis. Override with `@Name` any time.
- **Run controls**: pause, resume, stop (keeps your draft), retry;
  budgets on turns / tokens / USD / clock.
- **Approvals**: before sensitive actions the team shows prepare →
  preview → **approve** → submit. Nothing irreversible runs without you.
- **Schedules**: periodic runs (e.g. "every morning, brief me").
  They fire only while the app is open; missed ones offer catch-up.

## 9. Memory, files, and privacy

- **Memory**: per-project + global; the team recalls past decisions and
  stores new ones. Browse, edit, or delete in the Memory screen.
- **Artifacts** are always mirrored as `.md` files in your project folder —
  your data stays yours, in plain files plus a local SQLite store.
- **Privacy**: single user, local-first. No cloud account, no hosted
  thread storage. Only the provider/endpoint you chose receives prompt
  text, after secret-redaction; keys live in the keychain.

## 10. Troubleshooting

| Symptom | Fix |
|---|---|
| "No backend configured" | Add one key, start Ollama, or detect CLI agents (§4–5). |
| CLI agent not found | Launch Synapse from a shell once, or add the CLI's dir to your shell PATH; then Detect again. |
| CLI login expired | Re-login in a terminal, then Retry the run. |
| Local model not detected | Check `http://localhost:11434` (Ollama) / `:1234` (LM Studio) answers; add it as a custom endpoint. |
| Search returns little | Add a provider key (§7) or check `SEARXNG_URL` reaches a JSON-enabled instance. |
| MCP server fails to start | Run its command in a terminal first; check env vars; narrow to fewer tools. |
| Runaway cost | Lower the USD/token budget; give specialists cheaper models. |
| App won't start / second copy | Only one instance runs — check the tray icon; logs live in the app data dir (see Settings → About). |

Need more? In-app help (`?` / Command palette) covers every screen, and
`docs/` in the repo documents the architecture, protocols, and milestones.
