# Synapse: UI, avatars and motion

Built with GPUI Kit components. Before building any widget, check GPUI Kit's component list and
`examples/` (chat, stream-markdown, dock, settings, sidebar, command palette, notifications, charts).

## 1. Main window layout (Grok Bot style)

```
┌──────────┬───────────────────────────────────────────┬───────────────────┐
│ Sidebar  │  Header: project name · run mode · status │  Right panel      │
│          │  avatars of team (live state) · budget    │  (tabs, resizable)│
│ Projects │───────────────────────────────────────────│  - Team (members, │
│  ▸ pinned│                                           │    add agent)     │
│  ▸ all   │   Team room (virtualised message list)    │  - Sources        │
│          │   - grouped by agent, threaded replies    │  - Artifact       │
│ Agents   │   - typed message styles (§3)             │  - Activity       │
│  roster  │   - inline tool/approval/source cards     │  - Map (graph)    │
│          │                                           │  - Scheduled      │
│ ──────── │───────────────────────────────────────────│                   │
│ Settings │  Composer: @mention, mode picker, budget, │                   │
│          │  send ⇄ stop morph, queued follow-ups     │                   │
└──────────┴───────────────────────────────────────────┴───────────────────┘
```
GPUI Kit dock + resizable panels; panel sizes and open tabs persist per project. Command palette
(Cmd/Ctrl-K) for every action. Keyboard: Cmd/Ctrl-N new project, Cmd/Ctrl-Enter send, Esc stop, Cmd/Ctrl-1..5 panel tabs.

## 2. Screens

1. **Onboarding** (first launch): welcome → name your lead agent → choose its look (avatar editor) →
   add a provider (catalog picker + key, or detect local models/CLIs) → choose a team template → the
   lead introduces itself in the first project. Skippable; resumable.
2. **Project** (above). Empty state: goal input + template picker + example prompts.
3. **Agent editor** (sheet): avatar editor, name, role, instructions (markdown editor), backend picker
   (provider/model or CLI), params, tools & MCP toggles, permission rules, notes, Activity tab.
4. **Persona library** (dialog): sources (collections), search, category filter, preview, import (one or many).
5. **Settings** (page with sections): Providers (catalog list with search, key status, test), Models
   (defaults per role: lead, member, selector, embeddings), CLI agents (detected, login state), Search
   (SearXNG URL, keys), MCP servers (list, add, registry browser, per-tool toggles), Permissions (rules
   table), Appearance (theme, density, font size, reduce motion), Background (keep running on close,
   launch at login, hotkey, notifications), Data (folders, export, clear), About/updates.
6. **Quick-ask window** (hotkey): one input, project selector, answer streams in a compact view; "Open in app".
7. **Tray menu**: Open Synapse · Active runs (with phase) · Pending approvals (n) · Pause all · Quick ask · Quit.

## 3. Message styles

| Kind | Look |
|---|---|
| Say / Answer | bubble with avatar, name, role chip; markdown (streaming, word fade-in from GPUI Kit) |
| Ask | bubble with "→ @Agent" header; answered state links to the answer |
| Challenge | accent-bordered bubble quoting the challenged claim (click to jump) |
| Agree | compact one-line chip under the claim ("✓ Ana agrees") |
| Refuse | muted bubble with reason; shows lead's follow-up |
| Delegate | task card: assignee avatar, deliverable, progress, budget, open sub-thread |
| Finding | card with confidence meter and numbered citation chips (hover = source preview) |
| Status | thin checkpoint line, collapsible group ("Mira read 12 sources · 3 contradictions") |
| NeedUser / approval | prominent card: Approve once / Always allow / Deny, with diff or command preview |
| Done / synthesis | artifact card: open, diff vs previous version, export |

Three-layer detail toggle (from Muse): **Summary** (only findings/decisions), **Conversation** (default),
**Raw** (tool calls, prompts' metadata, token usage).

## 4. Avatars (the dots look)

Every agent has a floating 2D "dot": a soft blob drawn with GPUI `canvas` + `PathBuilder` Bézier paths,
gradient fill, subtle drop shadow; eyes and accessories are separate layers. No shaders, no images needed.

Customisation (stored as JSON on the agent):
- **shape**: bean (default), round, pear, cloud, star-ish, square-soft, drop, plus 4 animal-ish (cat, bird, bear, fish)
- **colour**: 12 preset palettes (base + highlight gradient) + custom
- **eyes**: diamond (default), dot, beady, sleepy, happy arcs, wide
- **mouth**: none (default), smile, o
- **accessories**: glasses, sunglasses, bow tie, beret, cap, headphones, crown, wreath (max 2)
- **seed**: deterministic random generator for "surprise me"

Default lead = green bean with two diamond eyes. New agents get a deterministic look from their name.

### Free avatar packs (alternatives to the blob, chosen per agent)

Besides our own blob, users can pick a ready-made style. All are free; licenses checked 2026-10-14 on
dicebear.com/licenses:

| Pack | License | Fit |
|---|---|---|
| **DiceBear** (dicebear/dicebear, MIT code): Notionists, Lorelei, Open Peeps, Thumbs, Shapes, Moods, Critters, Pixel Art, Pixelbot, Voxel Bot, Glass, Rings, Marbles, Planets, Clay, Blobs and 25 more | **CC0** (no attribution) | Default alternatives: people-like (Notionists, Lorelei, Open Peeps), bots (Pixelbot, Voxel Bot), abstract (Shapes, Marbles, Blobs) |
| DiceBear Bottts, Avataaars | Pablo Stanley's free license (personal + commercial) | Robot and cartoon-person agents |
| DiceBear Fun Emoji, Adventurer, Big Smile, Croodles, Micah, Personas, Toon Head, ... | **CC BY 4.0** (attribution needed) | Allowed; credited in About → Credits and in `assets/avatars/CREDITS.md` |
| microsoft/fluentui-emoji 3D | MIT | Role badges and accessories (🔬 researcher, ⚖️ critic, ✍️ writer) on top of any avatar |
| Google Noto animated emoji (googlefonts.github.io/noto-emoji-animation) | CC BY 4.0 (re-check before bundling) | Animated status reactions (thinking, celebrating), **only if** GPUI's `img` plays animated WebP/GIF in our pinned version; otherwise use the static frame |

How: DiceBear generates SVGs deterministically from a seed + options. A dev-only script
(`scripts/gen-avatars`, uses the DiceBear CLI via npx at build time, never at runtime) pre-renders a palette
of variants per style into `assets/avatars/<style>/` (SVG, small), plus a JSON manifest with license and
author. At runtime GPUI renders them as full-colour images (`img` with SVG source, not the monochrome
`svg()` element). They get the same state animations as the blob (float, bounce, squash, dim, badges),
applied to the whole image; only our procedural blob has blinking eyes. Rejected: Multiavatar (custom
license, unmaintained), Lottie/Rive packs (no GPUI player; velato/rive-rs need a vello renderer), anything
without a clear license.

States and animations (all via GPUI `with_animation` + GPUI Kit springs):

| State | Motion |
|---|---|
| idle | slow float (±2 px, 4 s), blink every 3–7 s (random), squash on hover |
| thinking | gentle wobble + eyes look up-left/right |
| speaking/streaming | small bounce synced to delta arrival (debounced), mouth/eye pulse |
| working (tools/research) | orbiting tiny dots around the blob |
| needs-you | bounce + badge, colour pulse once per 2 s (stops after 10 s) |
| paused | dimmed, eyes closed, no motion |
| error | brief shake, red badge |

Avatars render at sizes 16 / 24 / 40 / 96 / 160 px; below 24 px only colour + eyes, no animation.
**Animations must stop** when the window is hidden, when the avatar is off-screen, when "reduce motion"
is on, and for idle avatars after 30 s (they settle to a still pose until something happens). This is
required for the idle-CPU budget (PERFORMANCE.md).

## 5. Micro-interactions

- Send button morphs into a stop square while the team is running; the draft is kept on stop.
- Messages slide/fade in (≤ 150 ms); delegate cards expand into sub-threads.
- Approval cards slide up and pause the stream visually (stream behind is dimmed).
- Citation chips pop on hover with a source preview popover.
- Team header avatars hop slightly when their agent starts speaking.
- Typing shimmer (GPUI Kit skeleton/shimmer) only for the agents actually generating.
- Toasts for background events when the window is focused; OS notifications when it is not.
- Map view: edges animate briefly when a new Ask/Delegate is sent.
All durations ≤ 250 ms except ambient avatar float; everything honours "reduce motion".

## 6. Typography

Synapse has two sides: a **playful team of characters** (the dots, their chatter) and **serious long-form
research** (sources, analysis, reports). The type system gives each side its own voice and keeps dense chat
easy to read. Not the usual Inter / SF / Roboto / Geist. All SIL Open Font License (free to bundle, credited
in About → Credits), variable fonts from github.com/google/fonts (`ofl/<family>`).

| Role | Font | Why it fits Synapse |
|---|---|---|
| **Brand & display**: wordmark, onboarding, project titles, empty states, agent names in large cards | **Bricolage Grotesque** (variable: optical size, width, weight) | A grotesque with ink-trap quirks and personality: it matches the bubbly avatars without being childish; optical sizing keeps it crisp from 14 px labels to 48 px titles. |
| **UI & chat**: every message, label, menu, button | **Atkinson Hyperlegible Next** (variable weight + italic, Braille Institute, 2025) | Designed for maximum character distinction (I/l/1, O/0, rn/m). Users read many agents' streaming messages for a long time; legibility is the priority. Warm and distinctive, rarely used in apps. |
| **Reports & artifacts**: synthesis, research reports, long findings in the Artifact panel and exports | **Newsreader** (variable: optical size, weight, italic; Production Type) | An editorial serif made for on-screen reading: research output reads like a publication, clearly separate from the chatter. |
| **Code, data, tool calls, citations ids, numbers in tables** | **Atkinson Hyperlegible Mono** (variable weight + italic) | Same family as the UI font, so chat and code sit together in harmony; legible digits for costs, tokens and data. |

Rules:
- Type scale (px): 12 · 13 · **14 (body/chat)** · 16 · 20 · 24 · 32 · 48. Line height 1.5 for chat and reports,
  1.25 for UI controls. Reports: max line length ~72 characters.
- Weights: UI 400 / 500 / 650 (no 300 or thinner at small sizes); display 600–800; agent names in bubbles 650.
- Tabular figures for numbers in tables, budgets and costs.
- Size: fonts are **subset at build time** to Latin + Latin Extended + common punctuation/symbols
  (`scripts/subset-fonts`, pyftsubset, dev-only), Bricolage pinned to the width default (drop the wdth axis),
  regular + italic only where used. Target ≤ 700 KB for all four. Other scripts (Arabic, CJK, Devanagari, ...)
  fall back to system fonts automatically.
- Loaded with GPUI's text system from embedded bytes at startup (`add_fonts`); Settings → Appearance offers
  "Use system font" for the UI role and a font-size slider.

## 7. Icons

**Phosphor Icons** (MIT, phosphor-icons): ~1,500 icons on a 256 grid with six weights (thin, light,
regular, bold, fill, duotone). Not Lucide, which GPUI Kit ships by default and most apps use.
Why it fits: Phosphor's soft, slightly rounded geometry matches the blob avatars, and its weights give a built-in
state language:
- **regular** for UI chrome (toolbar, menus, sidebar);
- **fill** for the active/selected state (current tab, toggled tool);
- **duotone** for empty states, onboarding and feature cards (the friendly, illustrated side);
- **bold** at 12–14 px where regular gets thin.

Rules:
- Use only the icons we need: `scripts/sync-icons` copies a list (`assets/icons/icons.toml`, name → weight)
  of SVGs into `assets/icons/phosphor/`; nothing else is bundled. Render with GPUI's `svg()` (monochrome,
  tinted by theme colour); duotone uses the SVG's own opacity layer.
- Map GPUI Kit's `IconName`s to Phosphor where the kit allows custom icon assets (follow GPUI Kit's docs for
  overriding its asset source); never ship both Lucide and Phosphor.
- Semantic icons for message kinds (UI.md §3) are fixed: Ask = chat-circle-dots, Challenge = lightning,
  Agree = check-circle, Refuse = prohibit, Delegate = arrow-bend-up-right, Finding = magnifying-glass-plus,
  Propose = lightbulb, Vote = hand-pointing, Steer = steering-wheel, NeedUser = hand-waving, Done = flag-checkered.
  (Check each exists in the pinned Phosphor version; pick the nearest if renamed.)
- Role badges on avatars use Microsoft Fluent Emoji 3D (MIT) as in §4, not Phosphor, so they read as character, not UI.

## 8. Themes

System / Light / Dark from GPUI Kit's theme system; one accent colour setting; avatar colours stay
the same in both themes. Fonts: system UI font + a monospace for code.
