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
1. **Sidebar toggle** (Lucide `panel-left`; Cmd/Ctrl-B).
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
- Empty states: Lucide illustration in a soft badge + gentle avatar idle; one clear primary action.

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

Roles (code: `theme::TextRole`, applied with `Typography::typeset`; views never set raw sizes or weights):

| Role | Size / leading | Font · weight | Use |
|---|---|---|---|
| Display | 32 / 1.15 | Bricolage · 800 | onboarding, hero moments |
| Title | 20 / 1.25 | Bricolage · 600 | page titles |
| Heading | 16 / 1.3 | Atkinson · 500 | card titles, breadcrumb |
| Body | 14 / 1.5 | Atkinson · 400 | messages, reading text |
| Label | 13 / 1.3 | Atkinson · 500 | controls, nav rows, setting names |
| Caption | 12 / 1.4 | Atkinson · 400 | hints, metadata, counts |
| Overline | 12 / 1.3 | Atkinson · 700 | section labels (PROJECTS) |
| Mono | 13 / 1.5 | Atkinson Mono · 400 | code, tool calls, ids |

Dark theme adds 0.05 to every leading (light text looks thinner). Numbers that change or line up (costs, counts,
percentages) use tabular figures (`.figures()`). GPUI has no letter-spacing, so none is used. All sizes scale with
the Settings → Appearance text-size setting.

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

**GPUI Kit's built-in Lucide set** (ISC), the one icon set; no second set is bundled. (Decision 2026-10-14: we
first planned Phosphor, then chose the kit's own icons for zero extra assets and native component support.)

Rules:
- Every icon we use is named once in `synapse-app/src/icons.rs`; views import from there, never pick ad hoc.
- GPUI Kit ships 101 icons by default. Others we need are embedded by name through
  `app_assets::ExtraIcons` (`icon_assets!`), which embeds only the SVGs listed (~1 KB each).
- Render with GPUI Kit's `Icon` / `Button::icon`; colour comes from the theme (no hard-coded fills).
- Lucide has one weight: show the active/selected state with colour or a filled background, not a weight swap.
- Semantic icons for message kinds (UI.md §3) are fixed in `icons.rs`: Ask = message-circle-more,
  Challenge = zap, Agree = circle-check, Refuse = ban, Delegate = corner-up-right, Finding = search,
  Propose = lightbulb, Steer = waypoints, NeedUser = hand, Done = flag.
- Role badges on avatars use Microsoft Fluent Emoji 3D (MIT) as in §4, not the icon set, so they read as character, not UI.

## 8. Themes

System / Light / Dark from GPUI Kit's theme system; one accent colour setting; avatar colours stay
the same in both themes. Fonts: system UI font + a monospace for code.

## 9. Onboarding (first launch, under 2 minutes)

A single window, full-bleed, no title-bar clutter (only window controls), five steps, one at a time with a
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
4. **Brains** (how agents think), pre-filled from background detection:
   - "Found on this computer": Claude Code / Codex / Gemini CLI / OpenCode (logged in ✓), Ollama / LM Studio models.
   - **Providers**: a small "Add custom" button opens a popup (provider list searchable from the catalog, or
     "Custom endpoint…" for any OpenAI-compatible server; API key; **Add**, **Add more**, **Close**). Added
     providers are listed below in a scrolling list, each removable. The first one also becomes the lead's model.
   At least one backend must work to continue; otherwise the step explains the simplest option for this OS.
5. **All set**: one line on who leads the team and what is connected. Press Start: the window expands into
   the main layout, a "First project" room opens with the default Research squad, and the lead introduces
   itself and asks what to look into. Team template, research options and goals are chosen later, in the room
   and in Settings.

Measure: time-to-first-run from launch ≤ 2 min for a user with one key or one logged-in CLI (acceptance test
uses mocks and asserts the number of steps and that nothing waits on network).

## 10. Brand mark

Three bean agents with diamond eyes: a green lead (`#34d27b`, the default avatar colour) and two specialists
(blue `#3d9df2` = default accent, amber `#f5a524`). Source art is in `assets/brand/` (`synapse-mark.svg` colour,
`synapse-tile.svg` app-icon tile, `synapse-mark-mono.svg` single colour). `scripts/gen-icons` renders the installer
icons and the tray bitmap from it. In the app the mark is drawn with `brand_mark::mark` (an `img` of the embedded SVG,
so it keeps its colours); it appears at the top of the sidebar and on the first onboarding step. The tray icon is the
mark, with a green dot while a run works and an amber dot when an agent needs the user.
