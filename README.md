# Synapse

A free, open-source **native desktop app** where a team of AI agents brainstorms, researches the web
in depth, analyses and re-analyses: together and in real time. Think xAI Grok Bot / OpenAI dots /
CopilotKit OpenDots, but local-first, any provider, and light enough that you forget it is running.

- **A team, not a chatbot.** Put a question in a project; specialist agents research it, ask each other
  questions, challenge, refuse, delegate and converge on a revised analysis while you watch or interject.
- **Deep research, not surface search.** Breadth/depth research loops, multi-perspective questioning
  (STORM-style), cited sources, critique-and-revise rounds.
- **Any model.** Every provider in [models.dev](https://models.dev) via API key, local models
  (Ollama, LM Studio, llama.cpp), or the coding CLIs you already have installed (Claude Code, Codex,
  OpenCode, Gemini CLI, ...) via the [Agent Client Protocol](https://agentclientprotocol.com).
- **Tools with permissions.** MCP servers, file creation in a project workspace, sandboxed shell;
  allow / ask / deny rules and approval cards.
- **Lives in your tray.** Close the window and the team keeps working; notifications when they need you.
  No server, nothing runs when you quit.
- **Native and tiny.** Rust + [GPUI](https://www.gpui.rs) via [GPUI Kit](https://github.com/longbridge/gpui-kit).
  No Electron, no bundled browser.
- **Cute.** Customisable floating "dot" avatars for every agent, with micro-animations.

Status: specification stage. The build is driven by the docs in [`docs/`](docs/), milestone by milestone.

## Docs

| Doc | What it covers |
|---|---|
| [docs/PRD.md](docs/PRD.md) | Product: users, features, scope, what we copy from the reference apps |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, runtime/threads, data model, event flow |
| [docs/STACK.md](docs/STACK.md) | Every library we glue together, pinned versions, why |
| [docs/TEAM-PROTOCOL.md](docs/TEAM-PROTOCOL.md) | How agents talk to each other: message types, orchestration, limits |
| [docs/DEEP-RESEARCH.md](docs/DEEP-RESEARCH.md) | Research engine: search, reading, loops, citations, revision |
| [docs/UI.md](docs/UI.md) | Screens, avatar system, animations, interaction details |
| [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | RAM/CPU budgets and the rules that keep us inside them |
| [docs/MILESTONES.md](docs/MILESTONES.md) | Build order, tasks and acceptance tests |
| [docs/research/](docs/research/) | Raw research reports these decisions came from |
| [AGENTS.md](AGENTS.md) | Rules for anyone (human or AI) writing code here |
| [SETUP.md](SETUP.md) | Toolchain and optional API keys |

## License

MIT
