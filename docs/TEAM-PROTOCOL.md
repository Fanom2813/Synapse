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
    Done { summary },
}
```

Agents emit these by calling **team tools** (function calling), not by free-text parsing:
`say`, `ask_agent(agent, question)`, `answer(message_id, text)`, `challenge(message_id, reason)`,
`agree(message_id)`, `refuse(message_id, reason)`, `delegate(agent, task, deliverable)`,
`report_finding(text, citations, confidence)`, `propose(conclusion)`, `vote(...)`, `ask_user(...)`,
`done(summary)`. CLI agents get the same tools through Synapse's in-process MCP server.
If a model returns plain text without a tool call, it is recorded as `Say`.

## 3. Bus

- Per project: `broadcast::Sender<Event>` (capacity 1024) for UI/observers; lagging receivers resync from SQLite.
- Per agent worker: bounded `mpsc` inbox of `Envelope`s addressed to it (or to the room, filtered).
- `Event` = `Message(Envelope)` | `TextDelta { message_id, delta }` | `ToolCall{..}` |
  `ApprovalNeeded{..}` | `AgentState { agent, state }` | `RunState{..}` | `Budget{..}`.
- Text streams as `TextDelta`s of a provisional message; the final `Envelope` replaces it.

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
