# Synapse: deep research engine (`synapse-research`)

Goal: research that goes deep (follows leads, reads full sources, finds contradictions), not a
single search + summary. Ported, not invented: the loop from **dzhng/deep-research** (MIT), perspective
questioning from **STORM** (MIT), budget forcing ideas from **jina-ai/node-DeepResearch** (Apache-2.0),
planner/curation ideas from **GPT-Researcher** (Apache-2.0). Keep their license notices where code is ported.

## 1. Search providers (`SearchProvider` trait)

**Research works with zero keys out of the box.** Default search mode is `auto`, ported from ketch
(1broseidon/ketch, MIT, Go; port the logic and endpoints to Rust, keep its notice):

1. Anything the user configured, preferred in this order: own **SearXNG** / **Degoog** instance →
   keyed APIs (Brave, Serper, Exa, Linkup, Jina, Firecrawl, Tavily, SerpApi, Perplexity, Parallel, xAI;
   adapters ported from paperfoot/search-cli, MIT) → **provider-native web search** of the agent's model
   (Anthropic web_search/web_fetch, OpenAI web_search, Gemini grounding).
2. Otherwise the **keyless hosted chain**, first that answers wins:
   `parallel → exa → keenable → youcom → firecrawl → ddg`. Keys for these are optional and only lift caps.
3. External tools if on PATH: `ketch search --json`, `search` (search-cli) `--json`.

Modes (as in ketch): `auto` (first that answers), `multi` (fan out to every usable backend and merge
with **reciprocal rank fusion**, dedupe by URL, tag each hit with the engines that returned it), and
`random` (spread load across keyless backends to stay under their rate limits). Deep research uses
`multi` for planning queries and `random` for the many follow-up queries.

Keyless backends are free hosted tiers: they have rate limits and can change. So: per-backend
`governor` limits, a circuit breaker that skips a backend for 10 min after repeated 429/5xx, a
`doctor` check in Settings showing which backends currently answer, and a nightly-style live test
(`SYNAPSE_LIVE=1`) that fails loudly when a keyless endpoint changes. Tests use `MockSearch` fixtures.

**Fully keyless setup** = keyless search (above) + keyless reading (§2) + a model that needs no API key:
a local model (Ollama / LM Studio) or an installed CLI agent the user is logged into (Claude Code, Codex,
Gemini CLI, OpenCode), which uses their subscription. CLI agents also bring their own web search tools.

## 2. Reading

`fetch(url)` → cache check (SQLite `sources` by URL + ETag) → `reqwest` GET (per-host `governor`
limit, robots.txt via `texting_robots`, 15 s timeout, 5 MB cap) → content type:
- HTML → `dom_smoothie` (Readability) → `htmd` markdown;
- PDF → `pdf-extract`;
- failure / JS-only page → Jina Reader `https://r.jina.ai/<url>` (keyless 20 RPM, key optional).
Store markdown, title, hash; chunk (≈ 800 tokens) and embed lazily for retrieval.

## 3. The loop (per research sub-task)

```
research(query, breadth=B, depth=D, learnings=[], visited=[]):
    queries = llm.generate_serp_queries(query, learnings, n=B)      # distinct, specific
    for q in queries (concurrently, semaphore):
        hits = search(q); pages = read(top k not in visited)
        out = llm.extract(pages, q) -> { learnings[≤5, each with source ids], follow_ups[≤B] }
        if D > 1: research(next query from follow_ups, breadth=ceil(B/2), depth=D-1, ...)
    return dedup(learnings), visited
```
Defaults: B=4, D=2 (user can choose "quick" 2×1, "standard" 4×2, "deep" 6×3). Each agent's loop
reports `Status` checkpoints and finishes with a compressed `Finding` (≤ 300 words + citations).
Budget forcing: when the sub-task budget hits 90 %, stop searching and answer with what is known
("beast mode"), marking gaps.

## 4. Perspectives (STORM) for Brainstorm and Deep research modes

1. Lead searches the topic broadly and lists 4–6 **perspectives** (stakeholders/disciplines), mapping
   them to team members (or proposing new personas the user can add with one click).
2. Each member, in its perspective, asks the "expert" (a grounded researcher loop) questions; the
   Q&A is visible in the room.
3. Lead builds an **outline** (mind map) from all findings; Writer drafts sections with citations.

## 5. Analysis, critique, revision

- Analysis: members apply assigned frameworks to findings; output structured sections.
- Critique: Critic + Fact-checker review the draft: unsupported claims, contradictions between sources,
  missing perspectives, weak reasoning. Each issue is a `Challenge` linked to the paragraph.
- Revise: authors answer each challenge (fix, rebut with evidence, or mark as open question).
  New artifact version + changelog + diff view.
- Fact-check pass before final: every claim with a citation is checked against the stored source text
  (quote must exist); failures are flagged.

## 6. Output

Report artifact (markdown) with: executive summary, findings by question, analysis, disagreements and
how they were resolved, open questions, numbered sources (title, URL, date fetched). Sources panel
lists every source with which agents used it.

## 7. Tests

All loop logic runs against `MockLlm` (scripted responses) + `MockSearch` + `wiremock` pages.
Live smoke test (`SYNAPSE_LIVE=1`) runs a "quick" research against a real provider.
