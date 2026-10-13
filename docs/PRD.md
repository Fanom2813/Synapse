# Synapse: product requirements

## 1. Why

The owner could not find a native desktop app for working with a **team of agents**. Existing tools are
either single-assistant chat, web apps, or always-on servers (OpenClaw, Grok Bot, dots) that run in the
cloud 24/7. Synapse is a local desktop app for **brainstorming, deep research, deep analysis and
revised analysis** by a team of agents that talk to each other in real time.

## 2. Reference products (what we copy)

Full inventory: [research/reference-apps.md](research/reference-apps.md).

| Reference | What we take |
|---|---|
| **xAI Grok Bot** | Roster of named bots in a left sidebar; **chief-of-staff** bot over specialists; bots message each other and share **group channels** with a members panel; the conversation is the operating log (requests, checkpoints, questions, approvals, results); approvals before sensitive actions (prepare → preview → approve → submit). |
| **OpenAI dots** | The **avatar**: small bubbly floating cartoon characters (default: green bean blob with two diamond eyes), customisable shape, colour, eyes, glasses, accessories; agent profile with **Activity** and **Scheduled** tabs, custom rules, Pause/Reset; three-tier action policy (allowed / ask / hand off); private per-agent notes. |
| **CopilotKit OpenDots** | Specialist agents with role, instructions, permitted tools; avatar + status pinned above chat; inline tool cards; **Approve & run / Decline** cards; per-agent MCP servers with per-tool toggles (read-only tools auto-run); save conversation results as pages; themes System/Light/Dark. |
| **CopilotKit OpenMuse / Meta Muse** | Send arrow morphs to a stop square while streaming, draft kept on stop; visible follow-up queue; main chat + side chats; Activity with pause/resume/cancel/retry; **Ideas** surface backed by sources; three-layer status (summary / steps / raw). |
| **composio open-dot** | Architecture: closing the window keeps agents running until Quit; schedules and triggers only run while the app is open; missed runs skipped (we: offer catch-up). |

What we **drop** (needs a 24/7 server): cloud computers, work while the machine is off, inbound webhooks,
chat channels (Telegram/WhatsApp/Slack/Teams/SMS: none in v1), multi-user admin, hosted thread storage.

## 3. Users and jobs

Single user on their own computer (macOS, Windows, Linux).

1. **Brainstorm** a topic with a team: diverse perspectives, idea generation, critique, ranking.
2. **Deep research** a question: many sources, followed leads, cited findings, contradictions surfaced.
3. **Deep analysis**: structured analysis (SWOT, first principles, risks, options) by specialists.
4. **Revised analysis**: the team critiques its own output (red team), revises, and shows what changed.
5. **Produce artifacts**: reports, notes, files in the project folder (with permission).
6. Let it **run in the background** and get notified when done or when an approval is needed.

## 4. Core concepts

- **Project**: a workspace with a goal, a team (agents), a folder on disk, conversations, sources,
  artifacts, memory. Several can exist; each has one **team room** plus optional side chats.
- **Agent**: name, role, persona/instructions, avatar, backend, tools, permissions, private notes.
  Backend is one of:
  - **API model**: any provider/model from the catalog + a key (or local Ollama/LM Studio/llama.cpp).
  - **CLI agent**: an installed CLI driven over ACP (Claude Code, Codex, OpenCode, Gemini CLI, Goose,
    Copilot CLI, Cursor agent, ...), using the user's existing subscription/login.
- **Lead agent (chief of staff)**: one per project. Plans, assigns, picks who speaks, decides when the
  team has converged, writes the synthesis. The user can talk to the lead or to any agent directly.
- **Run**: one execution of a goal by the team (brainstorm / research / analysis / revision), with a
  budget (turns, tokens, USD, wall clock), live status, pause/resume/stop, and an activity log.
- **Message**: every utterance between user and agents is typed (see TEAM-PROTOCOL.md) and persisted.
- **Source**: a fetched web page / document with URL, title, extracted markdown, and who cited it.
- **Artifact**: a markdown report or file produced by the team, versioned (revised analysis keeps diffs).

## 5. Features (v1)

### 5.1 Team room and agent-to-agent communication (the core)
- Real-time stream of all agents' messages in one room, each with avatar, name, role, live status.
- Agents can **ask** another agent, **answer**, **challenge**, **agree**, **refuse** (with reason),
  **delegate** a task, **report a finding** (with citations), **propose** a conclusion, and **request
  the user**. Shown as distinct message styles (e.g. a challenge shows what it responds to).
- Several agents can work in parallel (research subtasks) while the room shows their progress.
- User can interject at any moment, @mention an agent, pause/resume/stop the run, change the budget,
  add/remove an agent mid-run.
- **Conversation map**: a graph view of who asked whom and delegated what (nodes = agents, edges = messages).
- Run modes as presets: **Brainstorm**, **Deep research**, **Deep analysis**, **Revise** (critique the
  current artifact and produce a new version with a changelog), **Free discussion**.
- Final **synthesis** artifact with citations, dissenting opinions and open questions.

### 5.2 Agents and personas
- Create/edit/duplicate/delete agents; per-agent model or CLI backend, temperature/thinking level, tools.
- **Persona library**: browse, search, preview and import from public GitHub collections; also from a
  URL or local `.md` file. Built-in sources (all MIT unless noted):
  | Source | What | Use in Synapse |
  |---|---|---|
  | msitarzewski/agency-agents (~160k★) | ~300 rich personas (strategy, marketing, sales, design, engineering, specialized...) with name, emoji, colour, vibe | **Main source of team members** for brainstorm/analysis |
  | danielmiessler/Fabric (~44k★) | 266 "patterns" (`data/patterns/<name>/system.md`): analyze_claims, check_falsifiability, analyze_risk, analyze_paper, analyze_debate, extract_wisdom, create_idea_compass, rate_content... | **Analysis frameworks** members apply in Deep analysis / Critique phases; also importable as skills |
  | K-Dense-AI/scientific-agent-skills (~49k★) | Agent Skills for scientific research (literature, databases, analysis) | Skills for research-heavy teams |
  | wshobson/agents (~41k★) | 202 agents + 184 skills (Claude Code plugin layout) | Technical specialists + skills |
  | VoltAgent/awesome-claude-code-subagents (~26k★) | ~170 agents by category | Technical specialists |
  | 0xfurai/claude-code-subagents, lst97/claude-code-sub-agents, iannuttall/claude-agents | 138 / ~35 / ~10 agents | Extra specialists |
  | anthropics/skills (~181k★) | Agent Skills reference set; per-skill licenses | Import skills whose own license allows it |
  | f/prompts.chat (~173k★, ex awesome-chatgpt-prompts) | thousands of "act as" role prompts; GitHub shows license NOASSERTION | Import on user request only; never bundled until the license is confirmed |
  | contains-studio/agents | no license | Link only, never imported automatically |
  Each imported persona records repo, commit sha and license. A curated **starter set** (~25 personas picked
  from agency-agents + Fabric frameworks + Synapse's own Lead/Critic/Fact-checker/Writer) ships in `assets/`
  so templates work offline.
- **Agent Skills** (`SKILL.md` folders): attach skills to agents; loaded on demand.
- **Team templates**: e.g. "Research squad" (lead, 3 researchers, fact-checker, devil's advocate,
  writer), "Product brainstorm", "Investment analysis"; user can save their own.

### 5.3 Providers and models
- Provider catalog from **models.dev** (≈228 providers): snapshot embedded, refreshed daily in the
  background (ETag); shows capabilities (tools, reasoning, vision), context, price.
- Add a key per provider (stored in the OS keychain), test connection, pick default models per role.
- Custom OpenAI-compatible endpoints (base URL + key) and local servers (auto-detect Ollama on
  :11434, LM Studio on :1234).
- **Installed CLIs**: auto-detect from the user's login-shell PATH and the ACP registry; one-click
  "use as agent"; shows whether it is logged in.
- Cost tracking per run/agent/project from usage × catalog prices.

### 5.4 Deep research
See DEEP-RESEARCH.md. **Works with no keys at all**: keyless search chain + built-in page reading + a
local model or a logged-in CLI agent. Search providers: provider-native web search when the model has it, SearXNG URL,
Brave / Tavily / Exa keys; page reading built in (readability → markdown) with Jina Reader fallback.
Sources panel with dedupe, credibility notes, which agent used which source.

### 5.5 Tools, MCP and permissions
- Built-in tools: web search, read URL, read/write/edit/list files **inside the project folder**,
  run shell command (sandboxed, off by default), ask user, notes (memory), create artifact.
- **MCP**: add servers (stdio command or HTTP URL, OAuth supported), browse the official MCP
  registry, per-agent enablement, per-tool toggles. Read-only tools (per annotations) may auto-run.
- **Permission rules** (global → project → agent): `allow` / `ask` / `deny` per tool and pattern
  (path glob, command prefix, MCP tool name); deny wins. Plus natural-language rules checked by a
  small review model before risky actions (optional).
- **Approval cards** inline in the chat and as OS notifications: Approve once / Always allow / Deny,
  with a preview (diff for file writes, full command for shell).
- **Audit log** of every tool call with arguments (secrets redacted) and result.
- CLI agents' own permission requests (ACP `session/request_permission`) use the same cards.

### 5.6 Memory
- Per-agent private notes; per-project memory (facts, decisions); a "What Synapse knows about you" page.
- Full-text + semantic search across messages, sources, artifacts (SQLite FTS5 + sqlite-vec with
  small local static embeddings).

### 5.7 Background, tray and desktop integration
- Closing the window hides it; agents keep working; Quit from tray or Cmd/Ctrl-Q stops everything.
- Tray menu: open, active runs with status, pending approvals count, pause all, quick ask, quit.
- When hidden for a while, the window is **destroyed** (not just hidden) to free memory, and rebuilt on open.
- OS notifications: approval needed, run finished, agent asks the user, errors.
- Global hotkey opens a small **quick-ask** window (ask the lead of the last project, or start a run).
- Launch at login (opt-in). Single instance (second launch focuses the first).
- **Schedules** ("every weekday 8:00, research X news"): run only while the app is running; missed runs
  shown as "missed" with a one-click run-now; Scheduled tab.

### 5.8 Avatars, look and feel
See UI.md. Dots-style floating blob avatars per agent (shape, colour, eyes, accessories), state
animations (idle, thinking, speaking, working, needs-you, paused, error), micro-interactions,
light/dark/system themes, onboarding (name your lead, choose its look, add a provider, it introduces itself).

### 5.9 Export
Artifacts and full transcripts to Markdown/HTML/PDF (PDF via OS print); copy as markdown; open folder.

## 6. Later (not v1; tracked as `later` issues)
Voice calls (realtime speech, captions); live browser control panel with take-over; chat channels
(Telegram, Slack, ...) while the app runs; teach-by-demonstration routines; plugin marketplace;
mobile companion.

## 7. Non-goals
No server, no account, no telemetry by default, no cloud sync, no multi-user.

## 8. Quality bars
- Idle: ~0% CPU; memory budgets in PERFORMANCE.md.
- Nothing blocks the UI thread; 60 fps scrolling with thousands of messages.
- Every feature works with **mock providers offline** in tests; live providers only behind `SYNAPSE_LIVE=1`.
- Keys never in logs, transcripts or crash reports.
