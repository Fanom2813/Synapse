# Synapse: memory

Designs ported, not code: **mem0** (Apache-2.0, 67k★: extract facts after a conversation, then decide
ADD / UPDATE / DELETE / NOOP against existing memories), **Letta/MemGPT** (Apache-2.0: small always-in-context
"core" memory + large searchable "archival" memory, agent edits its own memory with tools), **graphiti/Zep**
(Apache-2.0: facts carry valid-from / invalid-at times, so outdated facts are superseded, not deleted),
OpenClaw / Claude Code (human-readable memory files the user can edit). basic-memory is AGPL: read only, never copy.

## 1. What is remembered (scopes)

| Scope | Holds | Visible to |
|---|---|---|
| **User profile** | who the user is, preferences, writing style, recurring interests ("prefers sources from 2025+", "works in fintech") | all agents, all projects |
| **Project memory** | goal, decisions, conclusions, accepted findings, open questions, glossary, constraints | agents in that project only |
| **Agent notes** | the agent's private working notes and lessons ("Scholar search works better than Google for this topic") | that agent (in any project it joins, unless marked project-only) |
| **Run digests** (episodic) | per-run summary: question, what was found, sources, outcome, cost | the project; searchable |
| **Archive** | everything else: all messages, sources, artifacts (already stored) | via search only |

No project memory leaks into another project unless the user promotes it to the user profile.

## 2. Storage

- Rows in SQLite `memories(id, scope, scope_id, kind (fact/preference/decision/lesson/digest), text,
  source_message_id, created_by, valid_from, invalid_at, confidence, pinned, created_at, updated_at)`,
  indexed in FTS5 and sqlite-vec (model2vec embeddings, lazy-loaded).
- Each scope is also **mirrored as a readable markdown file** (`<project folder>/.synapse/MEMORY.md`,
  `<data dir>/memory/USER.md`, `.../agents/<name>.md`), regenerated from the DB. If the user edits the file,
  the edits are parsed back (diff → ADD/UPDATE/DELETE). Easy to inspect, back up and version.

## 3. Writing memory

1. **Explicit**: agents call `remember(scope, text, kind)`; the user can say "remember that…" or use
   "Remember" on any message. Explicit writes from the user apply immediately; from agents they apply to the
   agent's own notes immediately and to project/user memory as **proposals** (see 4).
2. **Automatic extraction** after each run (and every N messages in long runs): a cheap model reads the
   run digest + new messages and proposes facts, then the mem0 step compares each against the most similar
   existing memories and returns ADD / UPDATE (supersede: old row gets `invalid_at`) / DELETE / NOOP.
3. **Never stored**: secrets, keys, passwords, anything from a page marked sensitive, raw web content as
   "facts" (findings keep their citations and live in project memory only after the team accepts them).

## 4. User control

- **Memory screen** (Settings → Memory and a Project → Memory tab): list by scope, search, edit, pin, delete,
  see where each memory came from (link to the message), history of superseded versions.
- **Review queue**: proposed project/user memories appear as a small badge; approve all / one by one. A
  setting allows auto-accept for project memory (default off for user profile).
- **Forget**: delete one, clear a scope, "forget everything from this run". Pause memory per project.
- Export with the project; never synced anywhere.

## 5. Recall (using memory in a turn)

Built by the context builder (TEAM-PROTOCOL.md §7) within a fixed **memory budget** (default 1,500 tokens):
1. **Core block**, always included: pinned memories + the top user-profile facts + the project's goal,
   decisions and open questions (kept short; the lead re-summarises it when it grows past its budget).
2. **Retrieved block**: hybrid FTS5 + vector search over memories, run digests, the agent's notes and
   accepted findings, using the current task as the query; top-k after de-duplication; superseded facts excluded.
3. Agents can search further with `recall(query, scope)` and `search_archive(query)` tools.
Every recalled item carries its id so the UI can show "used 3 memories" on a message, with links.

## 6. Housekeeping

- Consolidation when a scope passes ~200 active rows: merge near-duplicates, re-summarise (proposal, user can undo).
- Embeddings computed lazily in a background task with low priority; the model unloads when idle.
- Memory size is shown per scope; old run digests can be archived (still searchable, not in the core block).
