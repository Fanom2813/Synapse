# Synapse: UI, avatars and motion

Built with GPUI Kit components. Before building any widget, check GPUI Kit's component list and
`examples/` (chat, stream-markdown, dock, settings, sidebar, command palette, notifications, charts).

## 1. Main window layout (Grok Bot style)

```
┌──────────┬───────────────────────────────────────────┬───────────────────┐
│ Sidebar  │  Team strip: all members' avatars (state),│  Right panel      │
│          │  @-click to address one (status/budget →  │  (tabs, resizable)│
│          │  live in the title bar, §1b)              │                   │
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

## 1b. Window frame and title bar

The window draws its own frame (GPUI Kit `TitleBar`, client-side decorations): no plain OS title bar. The
top strip is 40 px, draggable everywhere except on controls, double-click zooms/maximises (platform default).
Native window controls stay native: macOS traffic lights inset on the left; Windows minimise/maximise/close
on the right with Snap Layouts hover; Linux uses GPUI Kit's client-side buttons (follow GPUI Kit's docs for
each platform).

```
┌─[● ● ●]─[≡]─[ Market study ▾ ]────[ ◐ Research · 2/5 · ●●●○○ ]────[ 🔍 ⌘K ]─[ 🔔2 ]─[ $0.42 ]─[ ◧ ]──┐
   traffic  side  project switcher     run pill (phase, progress,     command   approvals  budget  panel
   lights   bar                        live agent avatars)            palette   inbox      meter   toggle
```

Left → right (macOS order; Windows/Linux mirror the window buttons only):
1. **Sidebar toggle** (Phosphor `sidebar`; Cmd/Ctrl-B).
2. **Project switcher**: current project name + chevron; click opens a searchable popover of projects
   (recent first, "New project" at the bottom, Cmd/Ctrl-N). The title cross-fades when switching.
3. **Run pill** (centre, the heart of the frame): mode icon + current phase + progress, and a stack of the
   **live avatars** of agents working now (max 5, then +N); each avatar shows its state (§4) at 16 px. Click
   = run popover: phase timeline, pause/resume/stop, budget, open Activity. Idle state: "Start a run" with the
   mode picker. While running, a 2 px progress line runs along the bottom edge of the title bar.
4. **Command palette** button showing the shortcut (⌘K / Ctrl-K).
5. **Approvals inbox** (bell with count badge): pending approvals and agents asking for the user
   ("needs you"); click = popover list with approve/deny inline. The badge bounces once when a new one arrives.
6. **Budget meter**: spent / budget for the current run as a tiny ring + amount; turns amber at 80 %, red at 100 %.
7. **Right panel toggle** (Cmd/Ctrl-Alt-B).
8. Overflow (narrow windows): items 6 and 4 collapse first into a "⋯" menu; the run pill shrinks to icon + avatars.

Background indicators also appear here when relevant, small and quiet: browser bridge connected (globe),
offline (cloud-slash), catalog/update available (dot on the overflow menu). The window frame itself does not
glow or pulse; status lives in the run pill.

## 2. Screens

1. **Onboarding** (first launch): see §9.
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

## 5. Motion system and micro-interactions

Goal: everything feels smooth and alive, never slow and never busy. Use GPUI's `with_animation` and GPUI
Kit's animation helpers (springs, keyframes, stagger); read their docs for the pinned version first.

**Tokens** (one module, `synapse-app::theme::motion`; never inline numbers):

| Token | Value | Use |
|---|---|---|
| `instant` | 80 ms, ease-out | hover/press colour, focus ring |
| `quick` | 140 ms, ease-out | tooltips, chips, toggles, icon weight swap |
| `standard` | 200 ms, ease-in-out | popovers, menus, message enter, panel content swap |
| `emphasized` | 280 ms, ease-out-back (slight overshoot) | approval cards, onboarding steps, delegate expand |
| `spring.soft` | stiffness 170, damping 26 | avatars, drag/resize settle, list reflow |
| `spring.bouncy` | stiffness 260, damping 18 | send→stop morph, badge bounce, avatar hop |
| `stagger` | 30 ms per item, max 6 items | lists appearing, onboarding choices |

**Principles**
- Animate only position/translation, scale, opacity, colour and corner radius; never re-layout big trees per frame.
- Animations are **interruptible**: a new state starts from the current value (springs), no jumps.
- Enter = fade + 4–8 px rise; exit = faster fade (60 % of enter). Things move from where they come from
  (a delegate card grows out of the message that created it; a popover scales from its trigger).
- Respect frame time: 60 fps everywhere, 120 fps on ProMotion displays where GPUI supports it; nothing animates
  off-screen or while the window is hidden; idle settles to zero frames (PERFORMANCE.md).
- Reduce motion (OS setting or Settings → Appearance): replace movement with short cross-fades, stop ambient avatar motion.

**Micro-interactions (complete list; each is a small task, keep them consistent)**
- Buttons: hover tint (`instant`), press scale 0.97, release spring back; disabled fades to 50 %.
- Send button morphs into a stop square while the team is running (`spring.bouncy`); the draft is kept on stop.
- Composer grows smoothly with text (height spring); @mention and /skill menus pop from the caret.
- Messages: enter with fade + rise; streaming text uses GPUI Kit's word fade-in; the author's avatar hops
  once when it starts speaking; grouped consecutive messages merge without jump.
- Typing shimmer (GPUI Kit shimmer) only for agents actually generating.
- Delegate cards expand into sub-threads (`emphasized`); progress bars ease, never jump.
- Approval cards slide up with slight overshoot and dim the stream behind them; approving collapses the card
  into a small "✓ approved" chip.
- Citation chips pop on hover with a source preview popover after 300 ms hover intent.
- Steering: a short line animates from the steering agent's avatar to the target's in the team header.
- Map view: edges draw in when an Ask/Delegate is sent, nodes ease with `spring.soft`.
- Panels: resize with spring settle; open/close slides with content fading in after 50 ms.
- Tabs and segmented controls: the selection indicator slides between items.
- Title bar: run pill expands/contracts smoothly when phases change; progress line eases; badge bounce.
- Lists (projects, agents, sources): stagger in on first show only; reorder with spring.
- Toasts slide in from the bottom-right, auto-dismiss with a draining progress line; OS notifications when unfocused.
- Success moments (run finished, agent created): the lead avatar does a small celebratory bounce + confetti of
  3–5 tiny dots in the avatar's colour (≤ 600 ms, once, skipped with reduce motion).
- Empty states: duotone Phosphor illustration + gentle avatar idle; one clear primary action.

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

## 9. Onboarding (first launch, under 2 minutes)

A single window, full-bleed, no title-bar clutter (only window controls), one step at a time with a
progress row of small dots at the top. Every step: one question, big friendly visuals, keyboard-first
(Enter = continue, Esc = back), skippable, and **resumable** (state saved after each step). Transitions
between steps slide horizontally with `emphasized` + stagger for the choices. Nothing blocks on the network:
detection runs in the background from the first frame.

1. **Hello** (3 s of delight): the default green bean dot floats in, blinks, waves; headline in Bricolage
   Grotesque: "Meet your research team." One button: "Let's start". Small link: "I've used Synapse before → import".
2. **Your lead**: name field (prefilled suggestion, e.g. "Dottie") with the avatar reacting as you type
   (looks at the caret, nods on Enter).
3. **Its look**: avatar editor carousel: shape, colour, eyes, accessory; "Surprise me" shuffles with a bounce.
   Live preview large in the centre.
4. **Brains** (how agents think), pre-filled from background detection, shown as cards that appear as they are found:
   - "Found on this computer": Claude Code / Codex / Gemini CLI / OpenCode (logged in ✓), Ollama / LM Studio models.
   - "Add an API key": provider picker (search the catalog) → key field → live "Testing…" → ✓ with the model list.
   - "Try free, no keys": local model or logged-in CLI + keyless research (shows what is needed if nothing is found).
   At least one must work to continue; otherwise the step explains the simplest option for this OS.
5. **Research** (optional, collapsed by default): connect the user's browser (§DEEP-RESEARCH 2b), SearXNG URL,
   or search keys. Default keyless search already works, so this is "Make research stronger (optional)".
6. **Your team**: choose a template (Research squad, Product brainstorm, Investment analysis, Custom); the
   team's avatars pop in one by one with names and roles; tap one to swap the persona.
7. **First project**: goal input with 3 example chips ("Is there a market for X in Uganda?", ...). Press Start:
   the window expands into the main layout (onboarding card morphs into the project view), the lead
   **introduces itself and the team** in the room, then starts the run (or waits if the user prefers).
   Optional: "Keep Synapse in the menu bar/tray" toggle and hotkey hint shown as a toast after the first run starts.

Measure: time-to-first-run from launch ≤ 2 min for a user with one key or one logged-in CLI (acceptance test
uses mocks and asserts the number of steps and that nothing waits on network).
