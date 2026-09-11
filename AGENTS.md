# AGENTS.md

Guidance for agents (and humans) changing this repository. Keep it current:
when a change makes a line here wrong, fix the line in the same commit.

## What this is

KiiChat is a lightweight Cherry Studio-style desktop chat client for any
OpenAI-compatible endpoint: manage a few providers, fetch each one's model list
with one click, and chat with streamed Markdown answers. Single window, single
binary, no server.

## Stack (do not mix versions)

The upstream UI stack is published as a set that moves together; mixing sets
gives two incompatible copies of GPUI's types and fails to compile in confusing
ways.

| Crate | Cargo line | What this app uses from it |
| --- | --- | --- |
| `gpui-pre` | `gpui = { package = "gpui-pre", ... }` | The framework; renamed so `use gpui::` works |
| `gpui-pre-platform` | `gpui_platform = ...` | Window/application bootstrap per OS |
| `gpui-component` | `gpui-component` | Theme + palette, `Root`, `TitleBar`, `Button`, `Input`, `TextView`, scroll helpers |
| `gpui-kit-assets` | `gpui-component-assets = { package = "gpui-kit-assets", ... }` | Bundled Lucide icons |
| `gpui-ai` | `gpui-ai` | `PromptBar` (composer), `StreamingText` (streamed Markdown), `Orbs` (empty-conversation animation). Its `Chat` component is deliberately **not** used |

Rules:

- Bump these **as one set** (`cargo update --precise` per crate) and re-verify
  the window opens. Never bump one alone.
- `Cargo.lock` is committed and is the source of truth for the resolved set.
- `gpui_ai::init(cx)` once, before any window. It initializes gpui-component
  too, so never call `gpui_component::init` as well.
- Every window's first-level view must be `gpui_component::Root`.
- `gpui-ai`'s `Chat` was dropped because its message row is fixed: it always
  renders a role heading, and its action row only offers
  copy/regenerate/edit/feedback with icons. This app needs no role headings,
  text actions (复制 / 分支 / 重试), a red 失败重试 state, and a branch action —
  none of which the component exposes. The transcript is therefore built here.

## Layout

```
src/main.rs    application bootstrap, window options, Root wiring
src/app.rs     the whole view: sidebar, transcript, composer, settings pages
src/api.rs     OpenAI-compatible HTTP: /models, streaming /chat/completions, proxy modes
src/icons.rs   asset source: the bundled icons plus the two glyphs they lack
src/store.rs   providers, sessions, messages, theme, proxy; JSON persistence
scripts/       dev tooling: fake provider server + UI Automation / capture helpers
```

## How the pieces fit

- **The view owns all state.** `KiiChat` holds `Store`; the transcript is a
  `gpui::ListState` (variable-height rows, `FollowMode::Tail`) rendered by
  `render_row`. No message snapshot type exists any more.
- **Message identity is the stored `Msg.id` (a uuid).** Element ids derive from
  it; never synthesize ids from indices alone.
- **Streaming never runs on the UI thread.** `api::stream_chat` spawns a thread
  with its own current-thread tokio runtime (reqwest needs a tokio reactor;
  GPUI runs on smol) and pushes `StreamEvent`s over an `async_channel`. The UI
  task consumes them via `update_in`.
- **List invalidation is explicit.** A delta calls
  `transcript.remeasure_items(row..row + 1)`; structural changes call
  `sync_transcript`, which resets the item count and only follows the tail when
  the reader is already at the tail (`force_follow` is for session switches and
  sends).
- **The list element needs an explicit size.** `list(...)` is a custom element:
  without `.size_full()` it lays out to zero height and paints nothing.
- **Persist on transitions, not on deltas.** `Store::save` after a send
  completes, a session/provider/theme/proxy changes — never per SSE chunk.
- **Proxy modes are resolved per request** in `api::client`: `System` keeps
  reqwest's default (env + OS proxy), `None` calls `.no_proxy()`, `Custom`
  installs `reqwest::Proxy::all(url)`. An empty custom URL is an error, not a
  silent fallback.
- **Errors surface in the UI.** A failed reply is stored in `Msg.error`, shown
  inside the bubble with a red border, and its retry action becomes a red
  "失败重试" text button.
- **Palette overrides must be re-applied after every theme change.**
  `Theme::change` re-applies the active theme JSON, wiping color edits, so
  `apply_theme` changes the mode first, then writes the palette into
  `Theme::global_mut(cx).colors`, then `Theme::sync_base` + `window.refresh`.
  Token order in `colors_of` and both palette arrays must stay in step.
- **Two icons are embedded by this app.** The bundled component icon set is a
  curated 101-glyph subset with no branch and no pencil, so `src/icons.rs`
  wraps `gpui_component_assets::Assets` and serves `icons/kiichat-*.svg` from
  string constants. GPUI rasterizes SVGs as alpha masks tinted with the text
  color, so a stroke-based SVG needs no color handling. Reference them with
  `Icon::empty().path(icons::GIT_FORK_PATH)`; `gpui_component::IconName` only
  carries the curated set.
- **Actions are icon-only square buttons** (`复制` / `分支` / `重试`, plus `编辑`
  on user messages). Every icon-only control carries `.tooltip(...)` and
  `.accessibility_label(...)`, which is also how the verification scripts find
  it. A failed reply's retry grows a red `重试` label.
- **Editing a user message re-sends it.** Saving the inline editor rewrites the
  message, drops every reply after it, and launches a fresh completion.
- **Client-side decorations.** The window is created from
  `TitleBar::window_options()` and the root view emits `TitleBar` as its first
  child. Without it the title bar disappears and the window cannot be dragged.
  Controls placed *inside* the title bar receive no clicks (its drag region
  covers them), so the sidebar / theme / page toggles live in the toolbar the
  main pane renders above its content — visible on both pages and while the
  sidebar is folded away.

## Conventions

- Code, comments and docs in English. User-facing UI strings in Chinese.
- Comments explain constraints, not narration. No `TODO` left behind.
- Rust edition 2024: `Future` comes from the prelude, no `std::future::` paths.
- Keep dependencies deliberate: `reqwest` + `tokio` (rt) + `futures-lite` +
  `async-channel` for networking, `serde`/`serde_json` for state, `uuid` for
  ids, `directories` for paths. Nothing else without a reason.

## Build and run

```sh
cargo build                 # debug
cargo run                   # launch the window
cargo build --release       # shipped binary
cargo clippy --all-targets  # must stay clean
```

Config lives at `%APPDATA%\KiiChat\config.json` (Windows),
`~/.config/KiiChat/config.json` (Linux),
`~/Library/Application Support/KiiChat/config.json` (macOS). Deleting it resets
the app.

## Verifying a change

GPUI has no headless frame test here, so verify against the running app. A clean
compile proves nothing about layout.

1. **Launch and read the accessibility tree** (Windows):

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/uia.ps1              # full tree
   powershell -ExecutionPolicy Bypass -File scripts/buttons.ps1 -MinY 0  # buttons, decoded names
   ```

   `buttons.ps1` writes UTF-8 to `%TEMP%\kiichat-buttons.txt` because the
   console mangles Chinese; read that file. It is the fastest way to confirm
   that rows, actions and settings controls really rendered.

2. **Drive it without a keyboard.** `scripts/invoke.ps1 -Name <label>` invokes a
   named button through UI Automation (use `-NameB64` for Chinese labels, e.g.
   `6YeN6K+V` is 重试 — the base64 of the UTF-8 name; `scripts/buttons.ps1`
   prints the base64 of every button name it finds, so you never have to
   encode one by hand).
   `scripts/click.ps1 -X -Y` synthesizes a real click and sets DPI awareness
   first; without that a 200%-scaled desktop aims at half the intended point.

3. **Exercise the network path against a fake provider**:

   ```sh
   python scripts/mock_openai.py   # /v1/models plus a streaming completion on 127.0.0.1:18080
   ```

   Point a provider's Base URL at `http://127.0.0.1:18080/v1`, then check the
   server log for the request and `%APPDATA%\KiiChat\config.json` for the
   persisted streamed answer.

4. **Verify visually without eyes.** Capture, then measure:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/capture.ps1 -Out D:/tmp/shot.png
   ```

   The capture is the window at its true pixel size (the script sets DPI
   awareness), while UI Automation reports *screen* coordinates — subtract the
   window origin before sampling a control's pixels. Compare mean luminance
   between light and dark frames, and check a specific rect's pixels for the
   expected ink (a red retry label, a pale blue bubble).

Hard rules for verification:

- Never claim a UI change works from a clean compile alone.
- Never leave `target/kiichat.exe` locked: stop the running app before rebuilding.
- The app writes to the real user config. If you seed it for a test, say so and
  restore the previous file afterwards.

## Repository hygiene

- `target/` must never be tracked (`git ls-files | grep -c '^target/'` is 0).
- Commit after each verified change, as `type: summary` (`feat:`, `fix:`,
  `docs:`, `chore:`), and push.
- Keep README's feature list and the GitHub description/topics in sync with what
  the app actually does.