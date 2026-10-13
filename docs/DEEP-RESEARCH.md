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

## 2b. Research in the user's browser (opt-in)

Why: no per-query cost, real search engines (Google, Bing, Scholar, X, Reddit...) with the user's own
session, pages behind the user's logins/subscriptions, JS-heavy pages. Implemented as a `SearchProvider`
(`browser:google`, `browser:bing`, ...) plus `browser_*` tools (open, read page as markdown, click, type,
scroll, screenshot), all through one `BrowserBridge` trait with three backends; Settings → Browser picks one
and walks the user through setup:

| Backend | How it attaches | Needs | Notes |
|---|---|---|---|
| **A. Chrome remote-debugging, native** (preferred) | Synapse connects over CDP with `chromiumoxide` to the user's running Chrome after the user enables it in `chrome://inspect/#remote-debugging`; Chrome shows its own permission dialog and an "automated" banner | Chrome ≥ 144 (verify exact version), no Node, no extension | Same mechanism as ChromeDevTools/chrome-devtools-mcp `--autoConnect` (Apache-2.0, 53k★): read its source for the discovery/handshake and port it to Rust. |
| **B. Existing MCP servers** (zero code, works today) | Configure as a built-in MCP connector: `npx chrome-devtools-mcp --autoConnect`, or `npx @playwright/mcp --extension` with Microsoft's **Playwright MCP Bridge** extension (Chrome Web Store; tab picker, per-connection approval) | Node/npx | Fallback when A isn't available (e.g. Edge, older Chrome). Tools come through the normal MCP path and permission rules. |
| **C. Synapse extension + native messaging** (later) | Own small MV3 extension talking to the Synapse binary via native messaging (the Claude in Chrome model); DOM extraction ideas from nanobrowser (Apache-2.0) | Extension install | Covers Chrome, Edge, Brave, Firefox without debugging flags. Tracked as a `later` issue. |

Rules (enforced by the bridge, not left to prompts):
- Opt-in per project; agents only use tabs **they opened** in the Synapse window/tab group, never the user's other tabs.
- **Human pace**: one search per 3–8 s per engine (jittered), max 2 agent tabs at once, daily soft cap
  shown in Settings. Automated querying can trigger CAPTCHAs or temporary blocks and may conflict with a
  search engine's terms; the setting explains this. So "free" means no API cost, not unlimited.
- CAPTCHA, login wall, 2FA → keep it simple: pause only that sub-task (other agents keep working), bring
  the tab to the front, send one OS notification "Please clear the CAPTCHA in your browser" (click = focus
  the tab). Synapse detects when it is cleared (CDP navigation/DOM events, no polling) and resumes on its own;
  no extra button. If it is not cleared within 10 min, the sub-task switches to the next search backend.
- Domain rules: default block list (banking, password managers, email send pages, payments); typing into
  forms and clicking submit buttons always **ask**; agents never read or type passwords.
- Page content read from the browser is untrusted data (prompt-injection guard as in ARCHITECTURE.md §6).

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
