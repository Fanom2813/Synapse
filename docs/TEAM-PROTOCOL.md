# Synapse: team protocol (how agents talk)

The core of the product. Implemented in `synapse-core` on tokio; no agent framework.
Patterns from: Anthropic's multi-agent research system (orchestrator + parallel subagents returning
compressed findings), AutoGen SelectorGroupChat (selector picks next speaker, termination conditions),
STORM / Co-STORM (perspective discovery, moderator, shared mind map), multi-agent debate (bounded
rounds, judge, distinct personas against sycophancy).

## 1. Participants

- **User**: a bus participant like any agent; can address the room, the lead, or any agent.
- **Lead** (chief of staff): one per project. Owns the plan, the budget ledger, speaker selection in
  discussion phases, convergence decisions, the final synthesis.
- **Members**: specialist agents (personas). Each has an `allowed_delegates` list (default: all members).
- **Built-in roles** a template can include: Researcher, Critic / Devil's Advocate, Fact-checker
  (citation checker), Writer (synthesizer). They are normal agents with stock personas.

## 2. Messages

Every utterance is an `Envelope`, persisted before it is published:

```rust
struct Envelope {
    id, run_id, conversation_id, thread_id, reply_to: Option<MessageId>,
    from: Participant,               // User | Agent(id) | System
    to: Recipients,                  // Room | Agent(id) | Agents(ids) | Lead | User
    kind: Kind,
    body: String,                    // markdown
    citations: Vec<SourceRef>,
    attachments: Vec<AttachmentRef>, // images, files, artifacts, sources: by reference, never inline bytes
    priority: Priority,              // Normal | Steer | Interrupt (see §3b)
    usage: Option<Usage>,            // tokens + USD
}
enum Kind {
    Say,                              // ordinary contribution
    Ask { question },                 // expects an Answer from `to`
    Answer,
    Challenge { claim_ref },          // disputes a specific message/claim, must give reasons/evidence
    Agree { claim_ref },
    Refuse { reason },                // declines an Ask/Delegate; first-class, shown and logged
    Delegate { task, budget, deliverable }, // asks another agent to do work; creates a sub-task
    Finding { confidence },           // research result with citations
    Propose { conclusion },           // candidate conclusion for convergence
    Vote { on, stance },              // used in convergence/ranking
    NeedUser { question, options },   // escalates to the user (OS notification if window hidden)
    Status { phase, progress },       // short checkpoint ("read 12 sources, 3 contradictions")
    Steer { instruction },            // redirects the recipient's current work (see §3b)
    Done { summary },
}
```

Agents emit these by calling **team tools** (function calling), not by free-text parsing:
`say`, `ask_agent(agent, question)`, `answer(message_id, text)`, `challenge(message_id, reason)`,
`agree(message_id)`, `refuse(message_id, reason)`, `delegate(agent, task, deliverable)`,
`report_finding(text, citations, confidence)`, `propose(conclusion)`, `vote(...)`, `ask_user(...)`,
`steer_agent(agent, instruction, interrupt?)`, `propose_agent(brief)` (Agent Builder, needs user approval), `attach(path | url | artifact | source)` (adds an attachment to
the next message), `done(summary)`. CLI agents get the same tools through Synapse's in-process MCP server.
If a model returns plain text without a tool call, it is recorded as `Say`.

## 3. Bus

- Per project: `broadcast::Sender<Event>` (capacity 1024) for UI/observers; lagging receivers resync from SQLite.
- Per agent worker: bounded `mpsc` inbox of `Envelope`s addressed to it (or to the room, filtered).
- `Event` = `Message(Envelope)` | `TextDelta { message_id, delta }` | `ToolCall{..}` |
  `ApprovalNeeded{..}` | `AgentState { agent, state }` | `RunState{..}` | `Budget{..}`.
- Text streams as `TextDelta`s of a provisional message; the final `Envelope` replaces it.

## 3a. Attachments (images, files, data)

Agents and the user can send each other **images, files, tables and references** in real time.

- `AttachmentRef { id, kind: Image | File | Table | Artifact | Source | Screenshot, mime, name, bytes, hash }`.
  The bytes live once in the project's content store (`<project>/.synapse/blobs/<hash>`, de-duplicated by
  hash); messages, the bus and the database carry only the id. Nothing is copied per recipient.
- Where attachments come from: user drag-and-drop/paste, files an agent creates (with permission), browser
  screenshots, charts an agent renders (e.g. a Python/plot script via the shell tool, or a simple chart spec
  rendered with GPUI Kit charts), MCP tool outputs, fetched PDFs/images from sources.
- **Delivering to a model** (done by the context builder, per recipient):
  - Image → sent as an image block **only if the recipient's model supports image input** (models.dev
    `modalities.input` contains `image`). Otherwise it gets a text description generated **once** by a
    vision-capable model (cached on the attachment) plus OCR text if any. Images are downscaled to the
    provider's max dimensions before sending.
  - Text-like file (md, txt, csv, json, code) → inline if small (≤ 4k tokens), else a summary + the agent can
    `read_attachment(id, range)` / search it.
  - PDF/DOCX → extracted text (pdf-extract / reader pipeline), same rule; page images only for vision models on request.
  - Table/CSV → schema + first rows inline; full data via `read_attachment` or the shell tool.
  - CLI agents (ACP) → as ACP resource/image content blocks when the agent advertises support, else as a file
    path inside the project folder.
- UI: thumbnails in bubbles, image viewer, file chips (open/reveal), drag out to Finder/Explorer.
- Limits: 20 MB per attachment, 10 per message (configurable); executables are never attached for execution.

## 3b. Real-time delivery and steering

Agents do not only talk between turns; they can **redirect each other while working**:

- Every agent worker checks its inbox at **safe points**: before each model call and after each tool result
  (the same points where Claude Code / Codex take queued user messages). New messages addressed to it are
  added to the next model call, so a long research loop sees "@Mira: skip blog posts, only peer-reviewed
  sources" within seconds, without restarting.
- `Priority::Steer` (via `steer_agent`): delivered at the next safe point with a "new instruction from X"
  marker; the agent must acknowledge (short `Status`) and adapt. Who may steer whom: the user → anyone;
  the lead → any member; a member → only agents it delegated to (its sub-tasks). Others must `Ask` or
  `Challenge` instead. Steering is shown in the room ("Lead steered Mira: …").
- `Priority::Interrupt`: cancels the recipient's in-flight model call (CancellationToken), keeps partial
  output as a draft, and restarts its turn with the new instruction. Only the user and the lead can interrupt.
  Pausing the run is an interrupt for everyone.
- Asks can be **blocking** (`ask_agent(..., wait: true)`: the asker's sub-task waits for the answer, with a
  timeout, then continues with "no answer") or **non-blocking** (default: the answer arrives at a safe point).
- Wake-up: an idle agent's worker task is parked on its inbox; a message wakes it (no polling). Agents that
  are not addressed and not selected cost nothing.
- Ordering: per-conversation monotonic sequence numbers; the UI and agents see messages in the same order.

## 3c. Efficiency (tokens, memory, CPU)

- **Address, don't broadcast**: messages go to named agents or the lead by default; `Room` messages reach
  agents only through the rolling summary unless they are @mentioned. This keeps the cost linear in team size,
  not quadratic.
- **References, not payloads**: envelopes carry ids for attachments, sources and long findings; the context
  builder resolves them per recipient within its token budget.
- **Prompt caching**: each agent's prompt is laid out stable-prefix-first (system/persona → tools → project
  core memory → plan → summary → new messages) and uses provider caching where supported (Anthropic cache
  control, OpenAI automatic caching, Gemini context cache), so repeated turns mostly hit the cache.
- **Cheap models for plumbing**: speaker selection, memory extraction, image captions and summaries use the
  project's "utility" model (default: cheapest capable model the user has), not the members' main models.
- **Compression**: sub-agents return ≤ 300-word findings; the lead re-summarises the room every phase.
- **Streaming**: deltas are coalesced per frame for the UI and are not persisted individually (only the
  final message is written), so the store does one write per message.
- **Bounded everything**: inbox capacity 64 per agent (backpressure: sender waits briefly, then the message
  is stored and delivered via resync), broadcast 1024, at most N concurrent model calls (semaphore).
- **Measured**: `--perf-probe run5` covers a 5-agent run with attachments and steering; token cost per run is
  shown in the UI and recorded in the run digest.

## 4. Run state machine

```
Idle → Plan → (Brainstorm | Research | Analysis) → Critique → Revise → Synthesize → Done
                     ↑__________________________________|   (repeat Critique→Revise ≤ max_rounds)
any → Paused → (resume to previous)        any → Stopped        any → Failed(error)
```

Mode presets choose which phases run and their parameters:

| Mode | Phases | Notes |
|---|---|---|
| Brainstorm | Plan → Brainstorm → Critique → Synthesize | STORM-style: lead asks each member for a distinct perspective first, then round-robin idea generation; critic attacks; votes rank ideas. |
| Deep research | Plan → Research → Analysis → Critique → Revise → Synthesize | Lead writes research questions; members run the deep-research loop in parallel (DEEP-RESEARCH.md) and return compressed Findings. |
| Deep analysis | Plan → Analysis → Critique → Revise → Synthesize | Frameworks per member (first principles, SWOT, risk, economics, ...), drawn from Fabric patterns (analyze_claims, check_falsifiability, analyze_risk, analyze_proposition, ...). |
| Revise | Critique → Revise → Synthesize on an existing artifact | Produces a new artifact version with changelog and diff. |
| Free discussion | Selector-driven chat until Done or budget | |

Run persistence: phase, plan, ledger and every message are in SQLite, so a run interrupted by quit
can be resumed (`interrupted` status).

## 5. Who speaks next

Parallel phases (Research, Analysis): lead `Delegate`s sub-tasks; a `Semaphore` (default 3) limits
concurrent model calls; each sub-task is an independent worker loop.

Discussion phases (Brainstorm, Critique, Free): rule-based selector first, LLM selector second:
1. a pending `Ask` / `Delegate` addressed to an agent → that agent;
2. an unanswered `Challenge` → the challenged agent;
3. the user @mentioned an agent → that agent;
4. otherwise the lead's selector (cheap model): given agent descriptions + rolling summary, pick the
   next speaker or declare `converged`; no agent speaks twice in a row unless asked.

## 6. Limits (all configurable per project; defaults)

| Limit | Default |
|---|---|
| max concurrent model calls | 3 |
| max turns per discussion phase | 12 |
| critique/revise rounds | 2 (max 3) |
| delegation depth | 2 |
| per-agent tool calls per sub-task | 25 |
| run budget | USD 1.00 or 400k tokens (whichever first); warn at 80 %, pause at 100 % and ask the user |
| wall clock | 30 min, then pause and ask |

Budgets use usage reported by the provider × models.dev prices; CLI agents report usage when ACP provides it,
else they count turns only.

## 7. Context per agent turn

Never the full transcript. Each turn gets: persona/instructions, project goal, current plan and phase,
its own notes, messages addressed to it (full), the **rolling summary** of the room (updated by the
lead after each phase or every N messages), relevant retrieved snippets (FTS/vector), and the team
tool list. Sub-agents return compressed findings (≤ 300 words + citations), not their raw search logs.

## 8. Quality guards

- Distinct personas; the Critic's prompt requires at least one substantive challenge per round.
- `Agree` without new evidence is allowed but counts toward convergence only if the agent is not the author.
- Findings without citations are marked "unsupported" in the UI and by the fact-checker.
- Convergence = lead `Propose` + majority `Vote` agree, or turn limit; dissent is preserved in the synthesis.
- `Refuse` must carry a reason; the lead must reassign or accept the refusal.

## 9. User controls during a run

Interject (message to room or agent), @mention, pause/resume, stop, skip phase, change budget, add or
remove an agent, approve/deny tool calls, pin a message as "must address", promote a finding to the artifact.
