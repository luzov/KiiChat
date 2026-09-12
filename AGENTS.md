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
src/main.rs           application bootstrap, window options, Root wiring
src/app/mod.rs        view state, wiring, Render, free UI helpers
src/app/chat.rs       sidebar, transcript, model bar, title bar, message rows
src/app/settings.rs   appearance / network / provider settings pages
src/api.rs            OpenAI-compatible HTTP: /models, streaming chat, thinking deltas, proxy modes
src/theme.rs          light/dark palette tokens and apply_theme
src/icons.rs          asset source: the bundled icons plus the two glyphs they lack
src/store.rs          providers, sessions, messages (incl. thinking), ModelInfo, key encoding
DESIGN.md             product UI design system (palette, density, voice, anti-patterns)
scripts/              dev tooling: fake provider server + UI Automation / capture helpers
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
- **Three API shapes live behind one setting.** `store::ApiFormat` picks
  `/chat/completions`, `/responses` or `/messages`; `api.rs` owns the request
  body, the auth scheme (`Bearer` vs `x-api-key` + `anthropic-version`) and the
  delta extraction per shape. The serde names are written out per variant —
  `rename_all = "kebab-case"` would have produced `open-ai-completions`.
- **A config the app cannot read is moved aside, never ignored.** `Store::load`
  returns the store plus a warning; on a parse error the file becomes
  `config.json.invalid` and the UI explains it. Starting empty and then saving
  is how a single bad field would otherwise destroy every provider.
- **Fetched models are a menu, not a save.** `获取模型` fills `Editor::fetched`
  and the user ticks what to keep; only 保存 writes them into the provider.
- **The composer's own model menu is not used for selection.** gpui-ai renders
  it inside a popup layer; scrolling it with the wheel does nothing, so the
  composer is handed only the current model and this app renders its own
  searchable picker above the composer (`render_model_bar`).
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
- **A wide drag region swallows its siblings' clicks.** The app draws its own
  title bar (`render_title_bar`) rather than using gpui-component's `TitleBar`,
  which puts its children inside the element registered as
  `WindowControlArea::Drag`. Measured on Windows: with the drag area spanning
  the row (≈900px), clicks on the toggles beside it became window drags; with
  the area on the title text alone (~70px), they arrive normally. So the drag
  area covers only the `KiiChat` text, and the window controls use the areas
  the platform hit-tests itself.
- **Drag and double-click.** `Window::start_window_move()` is a no-op on
  Windows (the platform leaves the default empty impl), so window moving relies
  on that drag area; `scripts/drag.ps1` verifies a title drag moves the window
  by the pointer delta.
- **Measure windows with Win32, not the capture script.** `capture.ps1` used to
  restore the window before capturing, which un-maximizes it: a "did maximize
  work?" check through it always read the restored size. It now restores only a
  minimized window and prints the minimized/maximized flags. The controls
  themselves are platform hit-test areas, so verify them by clicking their
  centre and reading `IsZoomed` / `IsIconic` / the process list — UIA can drive
  the app's own buttons, but not those areas.

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

2. **Read text back, not just buttons.** `scripts/buttons.ps1 -All` lists every
   element with its accessible name and rect (UTF-8 into
   `%TEMP%\kiichat-buttons.txt`); `scripts/setvalue.ps1 -Name <label> -Value <v>`
   types into a field through UI Automation — that is how the model search was
   verified (typing `model-1` cut 16 rows to 7).

3. **Synthetic wheel input does not reach the app.** Measured with both
   `mouse_event` and `SendInput`: a 976px-tall list inside a 220px scroll area
   does not move, and neither does a page known to be scrollable. Verify scroll
   regions by dragging the scrollbar, or ask the user to roll the wheel; do not
   read "it did not scroll" as proof the container is broken.

4. **Drive it without a keyboard.** `scripts/invoke.ps1 -Name <label>` invokes a
   named button through UI Automation (use `-NameB64` for Chinese labels, e.g.
   `6YeN6K+V` is 重试 — the base64 of the UTF-8 name; `scripts/buttons.ps1`
   prints the base64 of every button name it finds, so you never have to
   encode one by hand).
   `scripts/click.ps1 -X -Y` synthesizes a real click and sets DPI awareness
   first; without that a 200%-scaled desktop aims at half the intended point.

5. **Exercise every API shape against the fake provider**:

   ```sh
   python scripts/mock_openai.py   # /v1/models plus a streaming completion on 127.0.0.1:18080
   ```

   `scripts/mock_openai.py` serves all three shapes and asserts each request's
   shape (Bearer + `messages`, Bearer + `input`, `x-api-key` + `max_tokens`), so
   a reply that arrives proves the request was built for that API. Point a
   provider at `http://127.0.0.1:18080/v1`, then check the server log and
   `%APPDATA%\KiiChat\config.json` for the persisted streamed answer.

6. **Verify visually without eyes.** Capture, then measure:

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

## Packaging and CI

- `src/main.rs` carries `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`:
  a release build is a GUI app and must not open a console beside the window.
  Debug builds keep stdout, which is where `eprintln!` diagnostics go.
- `.github/workflows/ci.yml` builds and runs `clippy -D warnings` on
  `windows-latest`; `.github/workflows/release.yml` builds all three platforms
  and attaches the binaries to a GitHub Release when a `v*` tag is pushed.
- Neither workflow runs on this machine, so a workflow change cannot be
  verified here: keep the steps conservative and check the first run on GitHub.
- The Linux job installs GPUI's windowing dependencies; if upstream changes its
  feature set, that list is the first thing to revisit.

## Open items (2026-09-12)

Only what has been observed, with the evidence that showed it:

1. **The model picker's wheel scrolling is unverified.** Synthetic wheel input
   never reaches the app here — `mouse_event` and `SendInput` both left a
   976px-tall list inside a 220px scroll area where it was, and the settings
   page (a scroll container) did not move either. Ask the user to roll a real
   wheel; if it does not scroll, look at the panel's parents, not the input
   path.
2. **The top of the window is a resize band.** gpui_windows computes frame
   thickness itself when the OS title bar is hidden, so clicks a few logical
   pixels below the top edge resize instead — measured when a stray click
   shrank the window to 314x50. Controls in a 34px title bar sit close to it.
3. **Only Windows has been launched.** macOS and Linux compile in CI (green on
   v0.1.0) but have never been run.
4. **API keys are XOR-obfuscated at rest**, not OS-keychain encrypted. Threat
   model is "don't paste config.json in chat", not multi-user isolation.
5. **`publish` was exercised once**, on the v0.1.0 tag: all four jobs green and
   three binaries attached. Nothing else about tagging has been tested.
6. **Reasoning strip is unverified against a live reasoning model.** The wire
   parsers and UI exist; DeepSeek-reasoner / Claude thinking have not been
   driven end-to-end on this machine.

### Closed this session (2026-09-12)

- Picker panel is left-aligned under the model chip (340px, not a right-edge
  sibling of the composer row).
- Thinking/reasoning deltas stream into `Msg.thinking` and render as a
  collapsible strip; open while streaming, collapsed after Done.
- API keys encode as `enc:v1:` + base64(XOR against `install.key`); plaintext
  keys from older configs migrate on load.
- `src/app.rs` split into `src/app/{mod,chat,settings}.rs` plus `src/theme.rs`.
- Palette moved off the DeepSeek-website clone onto the DESIGN.md tool blues.
- Close-button idle ink matches min/max (was invisible against the title bar).
- Model catalog is a right drawer with +/−; closing it saves. Models store
  optional `max_tokens` / `context_window` from `/models`; chat uses the
  model's limit when present, else the provider fallback (default 8192).

## Verification rules learned the hard way

- `scripts/buttons.ps1` overwrites its report file even when it finds no
  window, and prints how many rows it wrote. Read that line: suppressing the
  script's own output and reading the file can serve the *previous* run's dump.
- The hub-supervised process named `kiichat` runs the **release** binary.
  `cargo build --release` before judging a UI change, and check the build
  actually succeeded — a stale binary cost two verification rounds.
- A seeded config must be checked against `src/store.rs`'s field names; a
  config the app cannot parse used to start it empty (now it is moved to
  `config.json.invalid` with a notice).

## Repository hygiene

- The hub-supervised process named `kiichat` runs the **release** binary.
  Rebuild with `cargo build --release` before judging a UI change — a stale
  binary cost two verification rounds on 2026-09-11.
- `target/` must never be tracked (`git ls-files | grep -c '^target/'` is 0).
- Commit after each verified change, as `type: summary` (`feat:`, `fix:`,
  `docs:`, `chore:`), and push.
- Keep README's feature list and the GitHub description/topics in sync with what
  the app actually does.