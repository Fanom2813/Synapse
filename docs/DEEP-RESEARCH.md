# Synapse: deep research engine (`synapse-research`)

Goal: research that goes deep (follows leads, reads full sources, finds contradictions), not a
single search + summary. Ported, not invented: the loop from **dzhng/deep-research** (MIT), perspective
questioning from **STORM** (MIT), budget forcing ideas from **jina-ai/node-DeepResearch** (Apache-2.0),
planner/curation ideas from **GPT-Researcher** (Apache-2.0). Keep their license notices where code is ported.

## 1. Search providers (`SearchProvider` trait), tried in this order

1. **Provider-native web search** when the agent's model supports it (Anthropic web_search/web_fetch,
   OpenAI Responses web_search, Gemini grounding; exposed by genai). Free with the user's key.
2. **SearXNG** URL set by the user (JSON format must be enabled on that instance).
3. **Keyed APIs** the user added (Brave, Serper, Exa, Linkup, Jina, Firecrawl, Tavily, SerpApi, Perplexity,
   Parallel, xAI for X posts), adapters ported from paperfoot/search-cli (MIT). When several are
   configured, fan out in parallel and merge with **reciprocal rank fusion** (a URL returned by several
   engines outranks any single engine's top hit), as search-cli does.
   Installed CLIs: if `search` (search-cli) or `ketch` is on PATH, offer it as a provider (JSON output).
4. Best-effort fallback: DuckDuckGo Lite scrape (often bot-walled; never the only option advertised).

If none is configured, the first research run shows a setup card explaining the options (with a
one-line Docker command for SearXNG). Tests use a `MockSearch` with fixture results.

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
