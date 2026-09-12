# DESIGN.md — KiiChat

## 1. Objective

KiiChat should feel like a local instrument you keep open all day — quiet chrome, dense when you need density, never marketing itself at you. After any well-executed screen, the user should think “this knows what I’m doing,” not “this looks like every other chat window.” Quality bar: portfolio-level product UI for a single-binary desktop tool; every decorative choice must earn its space or go.

## 2. Product Context

- **What the product does:** A local, single-binary desktop chat client that talks to any OpenAI-compatible endpoint: manage providers, pick models, stream Markdown answers.
- **Who it’s for:** A Chinese-speaking developer or power user who already has API keys (DeepSeek, Moonshot, OpenRouter, Ollama, one-api). Mid-20s to 40s. Lives in terminals and editors. Wants to open the app and type, not onboard.
- **Adjacent brands (feel like these):** Linear (restrained product chrome), Claude Desktop (transcript as reading surface), Raycast / Warp (keyboard-forward tool density).
- **Distant brand (do not feel like this):** DeepSeek’s consumer web chat — it is a website talking to browsers with promotional spacing; KiiChat is a tool sitting beside an editor.
- **Cultural register:** Technical, calm, local-first. Serious without being corporate. Not playful, not aspirational SaaS.

## 3. Visual Foundations

### 3a. Color

KiiChat keeps a cool-blue family (chat tools are blue for a reason) but stops cloning DeepSeek’s website. The accent is ink-blue, not brand-splash blue. Pale blue washes exist only as *surfaces for conversation*, never as decoration.

**Light**

| Token | Hex | Role |
| --- | --- | --- |
| `--n-0` | `#FFFFFF` | Main canvas, assistant bubbles |
| `--n-50` | `#F7F8FA` | Title bar, sidebar, muted panels |
| `--n-100` | `#EEF0F4` | Hover surfaces, secondary fills |
| `--n-200` | `#E2E5EC` | Borders, separators |
| `--n-300` | `#C9CEDA` | Disabled ink, empty icons |
| `--n-500` | `#6B7280` | Muted body, hints |
| `--n-700` | `#3A3F4B` | Secondary ink |
| `--n-900` | `#1B1D22` | Primary ink |
| `--accent` | `#3B5BDB` | Primary actions, selection, caret |
| `--accent-soft` | `#E8EDFB` | Selected rows, user bubble wash |
| `--accent-softer` | `#F2F5FD` | Composer focus wash, code bg |
| `--danger` | `#D64545` | Failed reply border, delete confirm |
| `--success` | `#2F9E6B` | Optional: copy-success toast only |

**Dark**

| Token | Hex | Role |
| --- | --- | --- |
| `--n-0` | `#17181C` | Main canvas |
| `--n-50` | `#121316` | Sidebar / title bar |
| `--n-100` | `#1E2026` | Input, popover, elevated |
| `--n-200` | `#2A2D36` | Borders |
| `--n-500` | `#9AA1AE` | Muted ink |
| `--n-900` | `#E7E9EE` | Primary ink |
| `--accent` | `#6B85FF` | Primary (lifted for dark contrast) |
| `--accent-soft` | `#2B3350` | Selected rows, user bubble |
| `--danger` | `#F07178` | Failure |

**Usage rules**

- Accent fires on: primary button (send / 保存), selected session row, caret, focus ring, current model chip. Never as a large background.
- User messages: `--accent-soft` fill, max-width 82%, right-aligned. Assistant: transparent on canvas — the transcript *is* the page.
- Sidebar sits one step darker (light: `--n-50`) so the main canvas reads as paper.
- One filled primary button per screen. Everything else is ghost / text / secondary.

### 3b. Typography

GPUI / gpui-component uses the system UI stack via font-kit. Do not bundle a web font.

- **Display / UI face:** system UI (`Segoe UI Variable` on Windows, `PingFang SC` / `SF Pro Text` on macOS, `Inter` / `Noto Sans CJK` on Linux).
- **Body face:** same as UI face — one family, no pairing. Chat tools that pair serifs look like magazines, not instruments.
- **Code face:** system mono (`Cascadia Code` / `SF Mono` / `JetBrains Mono` if present).
- **Type scale:** `11 / 12 / 13 / 14 / 16 / 18 / 22` — minor third from 12. No 24+ display sizes inside the app chrome; the transcript is not a landing page.
- **Weight discipline:** 400 body, 500 labels and buttons, 600 only for the window title and empty-state headline. No 700 anywhere.

### 3c. Spacing & rhythm

- **Base unit:** 4 px.
- **Scale:** `2, 4, 8, 12, 16, 24, 32, 48`.
- **Chrome density:** title bar 34 px; sidebar 232 px (collapsed 0); session row height 28–32 px; ghost icon button 24×24; primary button 28–32 px tall.
- **Transcript rhythm:** bubble padding `8×12`; gap between message groups `16`; actions row gap `2` under content with `12` clearance above.
- **“Generous” here means:** empty space around the transcript column (max-width ~720–760 px centered when window is wide), not oversized cards. Settings forms: 16 px between fields, 24 px between groups.

### 3d. Component seeds

- **Button:** three variants only — `primary` (filled accent, one per screen), `ghost` (icon + optional short label), `danger-text` (failed retry, delete confirm). Never six equal-weight CTAs.
- **Card / container:** almost none. Settings groups are sections separated by hairline dividers and a 12–14 px section title, not shadowed cards. Popovers (model picker) get a 1 px border + popover surface, radius 8, no drop-shadow theater.
- **Iconography:** Lucide via `gpui-kit-assets` (stroke 1.5–2, 14–16 px). Two local SVGs (branch, pencil) must match that weight. Icon-only controls always carry tooltip + accessibility label.
- **Radius:** 6 (controls), 8 (bubbles, panels). No 12/16 “soft SaaS” corners.
- **Composer:** PromptBar keeps its place; height ~44–56 px; model chip sits *above* the input left-aligned (see Structure), not as a popup layer.

## 4. Accessibility

- **Text contrast:** body ≥ 4.5:1, muted ≥ 4.5:1 on its surface, large/UI ≥ 3:1. Dark muted `#9AA1AE` on `#17181C` is the floor.
- **Motion:** streaming text is content, not decoration — allowed. Theme toggle, sidebar collapse, picker open: 120–180 ms ease-out, no bounce. Reduced-motion: collapse to instant.
- **Focus indicators:** 2 px accent ring (offset 1) on keyboard focus for every interactive control; mouse focus may suppress the ring but never the hit target (min 24 px).
- **Alt / labels:** every icon-only button has `aria-label` matching its Chinese tooltip. Decorative orbs on empty state need no alt; empty-state headline + body are the accessible content.

## 5. Voice & Tone

- **Register:** technical, calm, Chinese product UI. Instructional, not friendly-marketing.
- **Sentence rhythm:** short clauses. Prefer verbs: 「获取模型」「新建对话」「失败重试」.
- **Words this brand uses:** 「添加供应商」「获取模型」「新建对话」「选择模型」「失败重试」「以此消息为起点新建会话」.
- **Words this brand refuses:** 「开启智能之旅」「赋能」「一键畅享」「无缝体验」「您的专属 AI 伙伴」「探索无限可能」.
- **Address:** 「你」 implied by verb phrases; no 「您」 unless a confirm dialog needs formality. No second-person marketing copy on empty states — teach the next action, not the brand.

## 6. Implementation Practices

- **Token format:** Rust constants in one module (`theme.rs` or a `palette` block): light/dark arrays aligned to `gpui_component::ThemeColor` slots, applied after `Theme::change` then `Theme::sync_base`. Token order is part of the contract — a unit test may pin length.
- **Component library:** `gpui-component` + `gpui-ai`; no second design system. Transcript and message actions stay custom (see AGENTS.md).
- **Image treatment:** no photography, no illustration packs. Empty state uses `Orbs` at 48–56 px or a single Lucide icon + type. Screenshots in `docs/` are evidence, not UI chrome.
- **Grid system:** fixed side column (232) + fluid main. Transcript content max-width 760 px, horizontally centered when the main pane exceeds 900 px. Settings content max-width 560 px, left-aligned in the main pane.
- **Motion rules:** `ease-out`, 120–180 ms; opacity + 4 px translate for picker; height for sidebar only if free; never animate transcript scroll (list owns it).
- **Platform:** Windows is the verification platform (200% DPI). Client-side decorations, 34 px title bar, drag region on title text only.

## 7. Anti-Patterns

- **No DeepSeek website clone.** Same pale-blue wash + #4D6BFE is the previous identity; the redesign’s reason to exist is tool-density, not consumer-web chrome.
- **No marketing empty states.** Empty chat teaches the next click (设置 → 添加供应商), not a slogan under a spinning orb.
- **No equal-weight CTA row.** Settings save is primary; fetch-models, delete, cancel stay ghost.
- **No emoji as UI decoration.** Icons are Lucide; copy is Chinese verbs.
- **No card grid for settings.** Sections + hairlines; forms are linear.
- **No reasoning dumped as raw JSON or interleaved without affordance.** Thinking is a collapsed strip, default closed after completion.
- **No 16 px rounded soft-sass corners or drop-shadow popovers.** Radius 8 max, border over shadow.
- **No feature-page language inside the product** (「探索更多模型」).

## 8. Decision-Making

1. **Transcript readability wins.** If chrome competes with the answer, shrink the chrome.
2. **Clarity over cleverness.** A cute layout that hides the model picker loses to a plain one that works.
3. **One primary action per screen.** If everything is filled accent, nothing is primary.
4. **Local-tool density over consumer-chat air.** Prefer 28–32 px rows over 40+ px “touch targets” designed for phones.
5. **Accessibility floor is not negotiable.** Accent may be retuned for contrast; contrast is not retuned for brand.
6. **Ship the smallest version of a distinctive move.** A thinking strip, not a thinking theater.

## 9. Workflow

1. Read Objective + Product Context + this Structure section.
2. Name the screen/state being designed (chat default, chat streaming, picker open, settings/…, failure).
3. List information in reading order; put the 90%-of-time state first.
4. Apply Visual Foundations tokens — no one-off hexes.
5. Anti-pattern pass: any CTA row, card grid, slogan, or emoji → rewrite.
6. Accessibility pass: labels, contrast, focus ring, min hit target.
7. Implement against `AGENTS.md` stack rules; verify with UIA + capture, not compile alone.
8. Update this file if a decision here was wrong — same commit as the code change.
