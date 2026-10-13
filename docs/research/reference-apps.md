# Research: "Grok bot", "Dots", "Open Dots", "Open Muse bot" (as of 2026-10-14)

## 0. TL;DR: what the names refer to

The four names match one news cycle from Aug to Oct 2026. Three big labs shipped closed "always-on agent with its own computer" products, and open-source clones followed within days.

| Spoken name | Most likely product | Type | Confidence |
|---|---|---|---|
| "Grok bot" | **Grok Bot** by xAI (beta, Aug 11 2026) | Closed, cloud. Desktop (mac/Linux) + iOS clients | High |
| "Dots" | **dots** by OpenAI (ChatGPT, launched DevDay Sep 29 2026, model "GPT-6 Astra") | Closed, cloud | High |
| "Open Dots" | **CopilotKit/OpenDots** (5.2k stars, MIT, TS), the open clone of OpenAI dots. Close runners-up: **composio-community/open-dot** ("Open Dot", 639 stars, Electron **Mac desktop app**) and **Anil-matcha/open-dots** (5.8k stars, Python, Telegram + Claude Code) | OSS | High (name), Medium (which repo) |
| "Open Muse bot" | **CopilotKit/openmuse** (4.9k stars, MIT, TS / Expo RN), the open clone of **Meta Muse** (Sep 8 2026). "bot" may also mean **CopilotKit/OpenBot** (6.4k stars, MIT), CopilotKit's open clone of Grok Bot | OSS | High (OpenMuse), Medium (OpenBot) |

Background/related (not what the user named, but the same category):
- **OpenClaw** (ex-Clawdbot/Moltbot), openclaw/openclaw, ~392k stars, MIT, TypeScript. This is a 24/7 gateway assistant on WhatsApp, Telegram and other chat apps. Meta said Muse was "heavily inspired" by it.
- **Hermes Agent**, NousResearch/hermes-agent, ~253k stars, MIT, Python ("the agent that grows with you").
- **"Dot" by New Computer** is the older (2023-24) personal-journal AI. It is unrelated to this cycle and has low relevance.

**Best design references for a local desktop clone:**
1. **composio-community/open-dot** is closest to the target architecture: an Electron Mac app with an embedded local server that keeps working in the background after the window closes, until you press Cmd-Q. Routines and triggers run only while the app is open.
2. **CopilotKit OpenDots / OpenMuse / OpenBot** are the richest UI and feature references.
3. **OpenAI dots / Meta Muse / Grok Bot** are the visual and UX targets: the avatar characters, the computer panel with "Take over", and the approval cards.

### Repo facts (GitHub API, 2026-10-14)
| Repo | Stars | License | Lang | Stack |
|---|---|---|---|---|
| CopilotKit/OpenDots | 5,207 | MIT | TS | Node 24, React, CopilotKit + AG-UI, TanStack AI, CopilotKit Intelligence (threads, closed service), SQLite (pages), OpenBot computer supervisor (Docker), WebRTC realtime speech, Channels SDK (Slack), Parallel search MCP |
| CopilotKit/openmuse | 4,901 | MIT | TS | Expo / React Native (iOS, Android, web), Hono API, PGlite/Postgres, Playwright Chromium worker, optional Docker Linux "computer", Google OAuth (Gmail/Calendar), CopilotKit Intelligence |
| CopilotKit/OpenBot | 6,449 | MIT | TS | Bun, Postgres, Docker per-bot computers (optional gVisor), CEL policy gateway, AG-UI "bring any agent", MCP, Composio |
| composio-community/open-dot | 639 | none declared | TS | Electron + Next.js, OpenAI Responses API (GPT-5.5, computer tool, web_search), Realtime voice, OpenRouter, Composio (1,500+ apps, triggers), Playwright/Chrome, E2B / Docker / local folder, node:sqlite, macOS Keychain vault |
| Anil-matcha/open-dots | 5,759 | MIT (per description) | Python | FastAPI, Postgres, Telegram bot, Next.js web UI, Claude Code in Boat sandboxes |
| openclaw/openclaw | 391,898 | MIT | TS | Node gateway daemon, channel adapters, macOS menu-bar app, iOS/Android nodes |
| NousResearch/hermes-agent | 253,461 | MIT | Python | self-improving skills, memory, multi-platform gateway |

---

## 1. OpenAI dots (the "Dots" target)

Sources: learn.chatgpt.com/docs/dots, datacamp.com/blog/openai-dots, techcrunch.com/2026/09/29/openai-launches-dots-its-bubbly-agentic-avatar, simonwillison.net DevDay live blog, pasqualepillitteri.it.

### Visual / avatar (most distinctive)
- Each dot is a **small, bubbly, floating cartoon character**, a "cute blob-like avatar". TechCrunch calls them "little floating cartoons". Simon Willison called it "very Muse-like".
- **Default character:** a **green bean-shaped blob with two diamond-shaped eyes**, nicknamed "Dottie". It is the default face across ChatGPT, Codex and Slack.
- **Customization:** name, plus **shape, color, eyes, glasses, accessories**, changeable any time. Demo examples:
  - "Alfred" is a **yellow pear wearing glasses and a bow tie**.
  - "Felipe" is a **blue cloud wearing a beret**.
  - Simon's dot was a pelican, so animal/pet shapes exist. Community posts say the desktop app offers "pets, characters, and colors".
- Launch slides showed four colorful characters next to a lowercase "dots" wordmark. The event branding used animated clouds of dots.
- Handle: naming the dot changes its handle from `@user-dot` to `@user-<name>`.
- No source documents the animation states. Clone recommendation: idle float/bob, blink, a "working" state (busy eyes or orbiting dots), a "needs you" state (bounce plus badge), a "listening/speaking" state on calls, and a "paused" state (dimmed, eyes closed).

### Chat / UI
- Chat-style conversation with the avatar. Send button and mic dictation. A phone-call button starts a voice call, and you can keep typing during the call.
- Dot **profile** with tabs: **Activity** (delegated tasks, progress, requests for input), **Scheduled** (active, paused and completed runs), Custom Rules, connected apps, and connected computer. A "•••" menu holds **Pause** and **Reset** (Reset deletes the dot, its memories and schedules).
- **Computer view:** cloud Chromium, Terminal and Files. **Take over**, then **Return control**. Sign-in uses a private credential form, so credentials go to the browser and never into the chat. Saved logins are optional, and reusing one needs confirmation.

### Agents & multi-agent
- One dot per user at launch. "Teams of dots" are planned. **Specialist dots** (legal, finance) are enterprise pilots with their own identity and credentials.
- A dot spawns **background agents** for parallel work: cloud threads, Work and Codex tasks.

### Memory
- Inherits relevant ChatGPT memory and keeps its own **private notes** about preferences, decisions and ongoing work. Notes carry across channels.
- Its own memories are deleted only by deleting the dot.

### Tools / plugins / MCP
- 4,000+ apps through the OpenAI plugin ecosystem (Gmail, Drive, GitHub, ...). Each app must be connected and permitted.
- Codex cloud environments are used for coding.

### Permissions / approvals
- Every action is classified as **allowed**, **needs approval**, or **hand off to you**.
- **Custom Rules**: per action, choose one of four behaviors: act without asking, act if pre-approved (that is, explicitly requested in the prompt), ask first, or hand off.
- **Auto-review** checks account-affecting and sharing actions and cannot be disabled. Password changes always stay with the user.
- A monitor can pause the dot. Before sharing private info with others, the dot checks permission.

### Proactive mode
- When idle, the dot does **read-only** research over connected apps to find ways to help. It cannot send, edit, or control a browser or computer in this mode.

### Scheduling
- Schedules are set in natural language ("each weekday 8am, include time zone + end date, confirm"). Runs are managed in the Scheduled tab. Otherwise the dot decides on its own when to pause and resume.

### Channels
- ChatGPT desktop, web and mobile; **phone calls** (you call it, it cannot call you); **Slack** (DM or @mention; it watches only channels you specify); **Microsoft Teams**; SMS (limited US beta).
- Different update types can be routed to different channels, for example progress to ChatGPT and decisions to Slack.

### Local computer
- Optional, off by default. One computer at a time, connected through the ChatGPT desktop app ("Allow access").
- Gives access to local files, local skills and Codex tasks, and a local browser fallback when sites block cloud browsers. It needs the machine **online with the app open**.

### Onboarding
- Desktop is preferred (mobile setup is limited).
- Steps: name the dot, choose its look, connect apps, then the dot **introduces itself**. Next: connect Slack and the computer.

### 24/7 dependencies
Everything core runs in the cloud and keeps going while your devices are off: the cloud computer, proactive research, scheduled runs, Slack, Teams and phone channels.

---

## 2. Grok Bot (xAI)

Sources: unite.ai launch article, thecreatorsai.com review, buildfastwithai.com, aitoolsreview.co.uk.

### Visual
- Each Bot has a **face/avatar**. The default is a **generated geometric shape**. Users can AI-generate a face, upload a still, or upload an animated GIF, in an editor with **Bot / Generate / Upload** tabs plus Reset (via a third-party guide).
- Grok's separate "Companions" (3D animated Ani and others) are a different product.
- Layout: a **left sidebar roster** of named Bots and conversations, which can be pinned or grouped into sections (for example "GTM"). A center conversation, and a right-hand **computer panel** showing the Bot's cloud desktop live. A "+" menu creates a new Bot or a shared channel. Shared channels have a **Members panel**.

### Features
- Each Bot has a name, title, description and avatar. Bots are "teammates" you message like coworkers.
- The **conversation is the operating log**: requests, short checkpoints, questions, approval requests, results.
- **Computer:** a persistent cloud desktop with browser, files and terminal. It works in apps with no API. When the Bot hits a login, CAPTCHA, 2FA or an ambiguous step, you take over. All Bots on an account share **one** machine.
- **Teach by demonstration:** show the Bot a task once and it saves the steps as a reusable **routine**.
- **Routines** run a skill on a schedule or on an event, and post results to the conversation.
- **Multi-agent:** a "chief of staff" Bot sits over specialists. Bots message each other, share threads, join **group channels**, and hand off work.
- **Memory:** per-Bot context across conversations. Bots learn preferences and edge cases.
- **Approvals:** sensitive actions (outbound email, form submits, commercial terms) stop for approval. The recommended pattern is prepare, preview, approve, submit.
- **Plugins:** Gmail, Calendar, Slack, Notion, Stripe, Drive, support desks, X. A team marketplace for skills and plugins exists on Teams plans.
- Clients: desktop (Linux build posted) and iOS. Android is coming.

### 24/7 dependencies
The shared cloud computer, routines, event triggers, and Bots working after you close the laptop.

---

## 3. Meta Muse (inspiration for OpenMuse)

Sources: about.fb.com/news/2026/09/introducing-muse-personal-ai-agent, nbcnews.com, memeburn.com, lennysnewsletter.com, techcrunch.com.

### Visual
- A **cuddly, doll-like character avatar**. The default, "**Jolly**", is **cream-colored with beady black eyes and an upward smile**.
- Fully customizable name, attire and personality. Zuckerberg's avatar "Agrippa" wears a toga and laurel wreath.
- Also sold as the "Muse charm", a Labubu/Tamagotchi-like keychain device.
- UI is "messaging-style": chat plus the avatar.
- Design director Alex Cornell describes a **three-layer status display** (for different levels of technical depth), a personalized **Ideas** surface, a **generative feed** (generative UI), and a **main chat plus side chats**.

### Features
- A cloud "Muse Secure VM" with its own browser. An **inline mini-browser widget** in chat shows live navigation.
- **Sentinel**, a second agent, approves, blocks or refers every outbound action.
- Proactive suggestions panel, spending dashboard with savings goals, health goals with checkboxes, Goals view, activity log, and memory files.
- Connectors: GitHub, Notion, PayPal, Walmart, Stripe Link checkout.
- Clients: web, iOS, Android, WhatsApp, and a Mac app with computer use (Files, Messages, Calendar, Notes, Mail; full app control rolling out).

### 24/7 dependencies
The cloud VM, Sentinel, and background tasks.

---

## 4. CopilotKit OpenDots (the "Open Dots" match)

Repo: github.com/CopilotKit/OpenDots (MIT, alpha, released ~Oct 1 2026). Web and mobile. **Template, not a product.**

- **Spaces & Pages:** a Notion-like workspace. Searchable page library with grid/list views and nested subpages. Visual editor with **slash commands**, undo/redo, autosave with progress, revision checks, and a Markdown source mode. You can save a conversation as a page. Each page has its own conversation.
- **Specialist Dots:** name, role, instructions, permitted tools, per-Dot research and memory permissions, a default Space for saved pages, and access to multiple Spaces.
- **Chat:** a continuous conversation with the **Dot's avatar and status pinned above messages**. Text and call controls sit close by. The timeline shows work updates, source links and call receipts. A side panel shows results or the computer.
- **Dot computers** (via OpenBot): per-Dot browser profile and files that persist across stop/start. The Computer panel has browser control, **human takeover**, files, terminal output and activity, with per-Dot browser/file/shell permissions. Live browser, saved file and terminal output render **inline** as tool cards.
- **Review before saving:** a HITL card with **Approve & save / Decline**.
- **Connections:** per-Dot MCP servers with per-tool on/off. Read-only tools auto-run. Anything else shows an **Approve & run** card.
- **Calls:** WebRTC realtime speech plus a separate compute agent, so work continues during the call. Call screen has a **live timer**, **separate user and Dot captions**, mic mute, speaker mute, and a **minimize to chat** view. Call receipts land in the timeline.
- **Slack:** via Channels SDK, with workspace and user allowlists. Mention a Dot, then continue in the thread.
- **Background work:** scheduled server-side turns in the original conversation, with pause and retry. Runs interrupted by a worker crash are marked "Interrupted" and offer "Retry after review".
- **Memory:** user-managed preferences. **Automatic Learning** turns conversation evidence into published skills.
- **Appearance:** System, Light and Dark themes. Dark is derived at build time from the light styles.
- Public web research via Parallel Search MCP (up to 5 sources, saved with links).
- **24/7 dependencies:** scheduled turns (server worker), Slack, CopilotKit Intelligence (cloud or local Docker), and the computer supervisor.

## 5. CopilotKit OpenMuse (the "Open Muse" match)

Repo: github.com/CopilotKit/openmuse (MIT, alpha, Sep 22 2026). iOS, Android and web via Expo.

- **Chat:** headless chat with streamed AG-UI events. **Send arrow morphs into a stop square inside the input pill** while replying, and the draft is kept when stopped. Visible **follow-up queue**. Inline rich cards for email, browser, PDF, plan and finance. Main conversation plus **side chats** with rename, archive, restore and replay.
- **Agent computer:** persistent Chromium profiles, a **"Take control"** console that opens the same browser session, an optional isolated Linux terminal (30 s command limit, saved receipts, no network), editable workspace files, and PDF transfer.
- **Activity:** durable task plans, progress, input requests, pause/resume/cancel/retry, approvals, saved receipts.
- **Ideas:** suggestions backed by source evidence; edit, accept or dismiss.
- **Goals & Tracking:** goals and milestones. Recurring page watches for changes, text availability or price thresholds, with deduplicated alerts and failure backoff.
- **Documents:** email attachment, then PDF, form fill, reviewed reply, and receipt. PDF viewer with paging and zoom.
- **Finance:** CSV import, spending summary, savings-goal action.
- **Gmail & Calendar:** OAuth. Every send or calendar change requires its own stored review.
- **Personal context:** editable name, **tone, avatar**, memories, background-update preferences, in-app notifications.
- Generative-UI demo: clarification choice chips, sourced exhibit cards, preference refinement, confirmed selection.
- **Roadmap (not built):** voice, device push, health and bank connectors, generated tools.
- **24/7 dependencies:** the task worker and recurring watches ("the host must remain running for background work").

## 6. CopilotKit OpenBot (possible "bot" match, Grok Bot clone)

Repo: github.com/CopilotKit/OpenBot (MIT).
- A **channel per coworker**. Group channels are created by putting 2+ Bots in "To:".
- Per-Bot pages: responsibilities (lasting goals), reach (Slack, Teams, SMS), memory sources, approvals.
- `/skills` (invoked with `/` in the composer; a `skill-creator` skill authors skills in chat), `/routines`, and settings for connected accounts, passwords, notifications, approvals and memory.
- **Computer per Bot** (container, browser profile, `/workspace`, shell). A **"Watch it work" screen plus Activity tab**.
- **Take the wheel**: help requested, control taken, control released. Bot actions are refused while a human is driving.
- **Policy gateway:** CEL rules, deny before allow, fail closed, full **audit trail**. Secrets are never put in the transcript.
- **Generative components** instead of prose, with a sandboxed component playground.
- Governed MCP (Drive, Notion, Parallel, Composio).
- **Routines:** 15-minute minimum interval, maximum of 20, automatically disabled after 10 consecutive failures.

## 7. composio-community/open-dot ("Open Dot", Mac desktop app; best architecture match)

- **Electron** shell starts a local Next.js server. **Closing the window keeps dots running in the background until Cmd-Q.** Data lives in `~/Library/Application Support/Open Dot`.
- Each dot has its own **persistent, logged-in Chrome profile**, a **Computer tab** live view, and **take over** for logins and CAPTCHAs.
- **Password vault** encrypted with a macOS Keychain key. Passwords are typed straight into the page, so the model never sees them.
- **Composio:** Gmail, Calendar, Slack, Notion, GitHub and 1,500+ apps. Reads run on their own; anything that sends, posts, pays or changes asks first.
- **Natural-language rules** ("when it wants to reply to an email, ask first"). A small review model checks each risky action against the rules and shows an **approve/deny card** in chat.
- **Voice calls** (OpenAI Realtime). Work continues after hang-up, and the call transcript lands in the chat.
- **Schedules** (for example a weekday 8am inbox/calendar brief) post to their own chat and send a **notification**.
- **Triggers:** Composio events wake a dot.
- **Multiple dots** that pass work to each other. Each remembers facts about you and **saves skills** for repeated tasks.
- Code execution on an E2B cloud computer, a Docker container, or a local folder.
- Model picker: OpenAI, plus OpenRouter (Kimi, DeepSeek, Qwen, GLM). Open models fall back to DOM/text-based clicking.
- **Explicitly local-only:** "Routines and triggers only run while the app is open. A routine that comes due while your Mac is asleep gets skipped."

## 8. Anil-matcha/open-dots
- A coding-agent workspace: Claude Code tasks in sandboxes, driven from **Telegram** or a web UI.
- Approval prompts for file writes, shell and git push. GitHub device-flow linking.
- `/schedule` cron (hourly, daily, weekdays, weekly). Encrypted credentials.
- Server-based (FastAPI + Postgres), so it is a weak reference for a desktop app.

## 9. OpenClaw (ex-Clawdbot / Moltbot), for completeness
- **Gateway daemon** (24/7) bridging WhatsApp, Telegram, Slack, Discord, Signal, iMessage and more to one assistant.
- Lobster mascot "Molty" with 🦞 branding.
- **macOS menu-bar companion app**, Voice Wake and Talk Mode, **Live Canvas** (agent-driven visual workspace), and nodes (camera, screen, device actions) on iOS and Android.
- Skills and plugins (ClawHub registry), browser control, cron jobs and heartbeats, workspace persona and memory files (SOUL.md, MEMORY.md and similar).
- Onboarding wizard (gateway, workspace, channels, skills). Many model providers, including local ones.
- **Almost everything in OpenClaw assumes the always-on gateway.** That is the part the clone drops.

---

## 10. Consolidated feature inventory for the clone (union, grouped)

**Chat / UI**
- Messaging-style chat with the agent's avatar and live status in the header.
- Streamed replies. Send button morphs to a stop square in a pill input. Queued follow-ups. Drafts kept on stop.
- Inline rich cards: browser live view, file, terminal output, email, PDF, plan, sources.
- Main chat plus side chats (rename, archive). Sidebar roster with pin and sections. Activity tab, Scheduled tab, Ideas/suggestions, Goals.
- Pages/Spaces editor with slash commands (OpenDots).

**Agents & multi-agent**
- Multiple named specialist agents, each with role, instructions and tools.
- A chief-of-staff agent delegates. Agents message each other. Group channels with members panel.
- Background sub-agents for parallel tasks.

**Memory**
- Per-agent notes and memory files, plus user-editable "what it knows about you".
- Learns preferences. Auto-learned skills.

**Tools / skills**
- Skills invoked with `/`. Skill-creator. "Teach by demonstration" turns a recorded flow into a routine.
- Web search, code execution, and file workspace.

**MCP / integrations**
- Per-agent MCP servers with a per-tool toggle. Read-only tools auto-run; write tools need approval.
- Composio-style connectors: Gmail, Calendar, Slack, Notion, GitHub, Drive.

**File system & permissions**
- Per-agent workspace folder.
- Three-tier rules (auto / ask / hand off, plus "if pre-approved"). Natural-language custom rules checked by a small review model.
- Approve/Decline cards. Audit log. Encrypted password vault in the Keychain, with credentials kept out of the model's view. Local computer access off by default.

**Scheduling**
- Natural-language schedules. Scheduled tab with pause, resume and retry. Interrupted-run handling. Event triggers. Watches with thresholds.

**Browser control**
- Persistent per-agent Chrome profile.
- Live view and **Take over / Return control**. Help request on login, CAPTCHA or 2FA.

**Channels**
- Desktop app chat. Optionally Slack, Teams, Telegram or WhatsApp. These only work while the app runs in the clone.

**Voice**
- Realtime call screen with timer, dual captions, mute, speaker mute and minimize-to-chat.
- Work continues during and after the call. Call transcript and receipt in the timeline. Mic dictation in the composer.

**Tray / background**
- Closing the window keeps agents running; quit with Cmd-Q (Open Dot). Menu-bar presence (OpenClaw).
- OS notifications for schedule results and approval requests.

**Onboarding**
- Name the agent, customize its avatar, connect apps, then the agent introduces itself.
- Then connect channels and the computer. Settings for API keys (encrypted).

**Settings**
- Providers and keys, model picker, theme (system / light / dark), notifications routing (which update types go where), approvals, memory, connected accounts, passwords, computers (stop/reset), pause/reset agent.

**Providers**
- OpenAI (Responses API, computer tool, Realtime), OpenRouter (Kimi, DeepSeek, Qwen, GLM), Anthropic, Google.

**Visual design to copy**
- **Avatar system:** a small floating bubbly 2D/3D cartoon blob with customizable shape (bean, pear, cloud, animal), color, eyes (diamond, dot, beady), glasses, and accessories (bow tie, beret, toga/wreath).
- **Defaults to emulate:** OpenAI's green bean with two diamond eyes, Meta's cream doll with black beady eyes and a smile, Grok's geometric-shape face.
- Avatar shows in the chat header, the roster and Slack. Status states: idle, working, needs-you, on-call, paused.
- Live-computer panel beside the chat.
- Micro-interactions: send-to-stop morph, approval cards that pause the stream, captions on the call screen, a progress checkpoint style instead of verbose output.

## 11. Features that depend on a 24/7 server (the clone drops or degrades these)

| Feature (product) | Local-only fallback |
|---|---|
| Cloud computer that works while your devices are off (dots, Grok Bot, Muse VM) | Local Chrome profile and workspace while the app or tray is running |
| Proactive idle research (dots), Ideas (OpenMuse) | Runs only while the app is open; catch up on launch |
| Scheduled routines and triggers (all) | Run while the app or tray is running; missed runs are skipped or caught up on launch (Open Dot skips them) |
| Inbound webhooks and event triggers (Composio, Slack events) | Polling while open, or disable |
| Slack, Teams, SMS, WhatsApp, Telegram, phone-number calls (dots, OpenBot, OpenClaw gateway) | Optional, only while running; in-app voice calls instead of phone |
| Monitoring watches with alerts (OpenMuse) | Check while open |
| Muse Sentinel and dots auto-review as cloud services | Local review model call |
| CopilotKit Intelligence thread persistence (OpenDots, OpenMuse, OpenBot) | Local SQLite |
| Multi-user, SSO, team admin (OpenBot) | Not needed |
| Bots coordinating overnight, chief-of-staff long jobs (Grok Bot) | Only while running |
