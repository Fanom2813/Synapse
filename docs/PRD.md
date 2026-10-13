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
- **Artifact**: a markdown report or file produced by the team, versioned (revised analysis keeps diffs), and
  always mirrored as a `.md` file in the project folder (ARCHITECTURE.md §3).

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
- **Skills** (open Agent Skills format, agentskills.io: a folder with `SKILL.md` + optional `scripts/`,
  `references/`, `assets/`; the same format Claude Code, Codex, OpenCode, Gemini CLI and Cursor read):
  - **Progressive disclosure**: only each skill's name + description sit in the agent's prompt; the agent
    calls `load_skill(name)` to read the full SKILL.md, then `read_skill_file` for references when needed.
    Scripts in a skill run only through the shell tool and its permission rules (ask by default).
  - **Where skills come from**: built-in starter skills (research methods, report writing, citation style,
    Fabric analysis patterns); import from GitHub collections (anthropics/skills, K-Dense scientific skills,
    wshobson/agents skills, others with a clear license) and from skill directories (e.g. skills.sh, ClawHub;
    verify they still exist and their licenses at build time); from a URL, zip or local folder; and
    **existing skills already on the machine** (`~/.claude/skills`, `~/.codex/skills`, `.agents/skills`),
    shown with their source and imported only when the user chooses.
  - **Scopes**: global, per project, per agent. Toggle per agent. Skill library screen: search, preview,
    enable, update from source, delete.
  - **Use**: agents pick skills themselves by description; the user can force one with `/skill-name` in the
    composer (slash menu lists enabled skills), as in OpenBot.
  - **Create**: a built-in `skill-creator` skill helps the user (or an agent, with approval) write a new skill
    in chat; editor with live preview; saved to the data dir as a normal skill folder (shareable).
  - **Learned skills** (as dots / OpenDots "automatic learning"): after a run, the lead may propose "save this
    as a skill" (e.g. a research procedure that worked); the user reviews and approves; never saved silently.
- **Create an agent from a prompt (Agent Builder)**: the user describes the agent they want ("a Ugandan
  agent who understands how people in Uganda think, talk and decide") and a built-in **Builder** agent
  researches and drafts it. Flow:
  1. **Clarify** (≤ 3 quick questions, skippable): e.g. which region/community (Buganda, Acholi, Ankole...,
     urban Kampala vs rural), which languages (English, Luganda, Swahili...), what the agent is for
     (market research, writing, advising), age/background if relevant.
  2. **Research** with the deep-research engine (DEEP-RESEARCH.md): culture, values, history, social norms,
     languages and common phrases, economy and daily life, current affairs, how decisions are made, what
     outsiders get wrong. Prefers local sources (local news, universities, writers from the place) and
     records every source.
  3. **Draft** the agent: name, role, persona/instructions (voice, values, how it reasons, what it knows,
     what it should not claim), a **knowledge pack** (a markdown reference file + key facts saved as the
     agent's notes in memory, with citations), suggested skills/tools, a model suggestion, and an avatar look.
  4. **Review**: the user sees the draft with its sources, edits anything, and can **test-chat** with it before saving.
  5. **Save** as a normal agent (exportable as a persona `.md` + knowledge folder; shareable).
  Rules for people/culture agents: grounded in sources, not stereotypes; reflects diversity inside the group
  (regions, generations, urban/rural) and says when views differ; never claims to be a real person or to
  speak for everyone; states it is an AI perspective built from research; the critic can challenge it like
  any member. "Refresh knowledge" re-runs the research later and shows what changed.
- **Agents creating agents**: during a run, the lead (or any member, if allowed) can call
  `propose_agent(brief)` when the team lacks a perspective ("we need someone who knows Ugandan mobile-money
  regulation"). The Builder runs in the background and the user gets an approval card with the draft;
  approve = the new agent joins the team. Never added without approval (a project setting can allow
  auto-approve for the lead). Also: "build a team for this goal" creates a whole team the same way.
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
local model or a logged-in CLI agent.
**Research in the user's own browser** (opt-in): agents can drive the user's real, logged-in Chrome/Edge/Brave
to search Google/Bing/any site with no API cost, read pages behind the user's logins and subscriptions,
and handle JS-heavy sites. Runs in a dedicated "Synapse" window/tab group, at a human pace, and hands
control to the user on CAPTCHAs or logins. See DEEP-RESEARCH.md §2b. Search providers: provider-native web search when the model has it, SearXNG URL,
Brave / Tavily / Exa keys; page reading built in (readability → markdown) with Jina Reader fallback.
Sources panel with dedupe, credibility notes, which agent used which source.

### 5.5 Tools, MCP and permissions
- Built-in tools: web search, read URL, read/write/edit/list files **inside the project folder**,
  run shell command (sandboxed, off by default), ask user, notes (memory), create artifact.
- **MCP**: add servers (stdio command or HTTP URL, OAuth supported), browse the official MCP
  registry, per-agent enablement, per-tool toggles. Read-only tools (per annotations) may auto-run.
- **Connectors gallery** (app integrations, the dots/Grok Bot "connect apps" step), all built on MCP so
  there is no per-app code:
  - **One-click remote connectors**: services that host an official MCP server with OAuth (candidates:
    GitHub, Notion, Linear, Atlassian Jira/Confluence, Slack, Asana, Sentry, Stripe, Cloudflare, HubSpot,
    Google Workspace/Drive/Gmail/Calendar where available). Click → OAuth in the browser → tokens in the
    keychain → tools appear. The list is curated in `assets/connectors.json` and **every URL is verified
    against the MCP registry / vendor docs when built**; entries that no longer exist are dropped.
  - **Local connectors** (stdio, run on demand): filesystem folders, Obsidian/Markdown vault, SQLite/Postgres,
    Git, browser (Playwright MCP, opt-in), Apple Notes/Calendar/Mail/Reminders on macOS, Office/Outlook on
    Windows where a maintained MCP server exists.
  - **Aggregators** for long-tail apps (thousands of apps behind one MCP endpoint): Composio, Zapier MCP,
    Pipedream; user brings their own account/key.
  - Each connector card: what it can read/write, which tools are read-only, which agents may use it,
    connection status, disconnect. Write tools default to **ask**.
- **Synapse as an MCP server** (other apps talk to the team): opt-in local server (stdio for Claude
  Desktop/Cursor/Zed configs, and localhost streamable HTTP with a token) exposing `list_projects`,
  `start_run(project, mode, prompt)`, `run_status`, `get_report`, `ask_agent`. Off by default; Settings shows
  copy-paste config snippets for popular clients.
- **Permission rules** (global → project → agent): `allow` / `ask` / `deny` per tool and pattern
  (path glob, command prefix, MCP tool name); deny wins. Plus natural-language rules checked by a
  small review model before risky actions (optional).
- **Approval cards** inline in the chat and as OS notifications: Approve once / Always allow / Deny,
  with a preview (diff for file writes, full command for shell).
- **Audit log** of every tool call with arguments (secrets redacted) and result.
- CLI agents' own permission requests (ACP `session/request_permission`) use the same cards.

### 5.6 Memory
See [MEMORY.md](MEMORY.md): what is remembered, who writes it, how it is recalled, and user control.

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
