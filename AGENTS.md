# AGENTS.md

Guidance for agents (and humans) changing this repository. Keep it current:
when a change makes a line here wrong, fix the line in the same commit.

## What this is

KiiChat is a lightweight Cherry Studio-style desktop chat client for any
OpenAI-compatible endpoint: manage a few providers, fetch each one's model list
with one click, and chat with streamed Markdown answers. Single window, single
binary, no server.

## Stack (do not mix versions)

The UI stack is published as a set that moves together; mixing sets gives two
incompatible copies of GPUI's types and fails to compile in confusing ways.

| Crate | Cargo line | Role |
| --- | --- | --- |
| `gpui-pre` | `gpui = { package = "gpui-pre", ... }` | Zed's GPUI, renamed so `use gpui::` works |
| `gpui-pre-platform` | `gpui_platform = ...` | Window/application bootstrap per OS |
| `gpui-component` | `gpui-component` | Theme, `Root`, `TitleBar`, Buttons, Inputs |
| `gpui-kit-assets` | `gpui-component-assets = { package = "gpui-kit-assets", ... }` | Bundled Lucide icons |
| `gpui-ai` | `gpui-ai` | `Chat` (virtualized transcript + composer), `PromptBar`, `StreamedContent` |

Rules:

- Bump these **as one set** (`cargo update --precise` per crate) and re-verify
  the window opens. Never bump one alone.
- `Cargo.lock` is committed and is the source of truth for the resolved set.
- `gpui_ai::init(cx)` once, before any window. It initializes gpui-component
  too, so never call `gpui_component::init` as well.
- Every window's first-level view must be `gpui_component::Root`.

## Layout

```
src/main.rs    application bootstrap, window options, Root wiring
src/app.rs     the whole view: sidebar, chat pane, settings page, streaming
src/api.rs     OpenAI-compatible HTTP: /models, streaming /chat/completions
src/store.rs   providers, sessions, messages, theme; JSON persistence
scripts/       dev tooling: mock provider server, UI Automation helpers
```

## How the pieces fit

- **The view owns all state.** `KiiChat` holds `Store` and hands `Chat` an
  `Arc<[ChatMessage]>` snapshot. `gpui-ai` components render snapshots only.
- **Message identity is the stored `Msg.id` (a uuid).** `Chat::set_messages`
  silently ignores a snapshot with duplicate ids, so never synthesize ids from
  indices.
- **Streaming never runs on the UI thread.** `api::stream_chat` spawns a thread
  with its own current-thread tokio runtime (reqwest needs a tokio reactor;
  GPUI runs on smol) and pushes `StreamEvent`s over an `async_channel`. The UI
  task consumes them via `update_in`, which is where the `&mut Window` needed
  by `Chat::set_messages` comes from.
- **Persist on transitions, not on deltas.** `Store::save` after a send
  completes, a session/provider changes, or the theme changes — never per SSE
  chunk.
- **Errors surface in the UI.** Failed assistant messages are stored with
  `Msg.error`, rendered as a failed bubble, and marked `retryable(true)` so
  Chat's Retry button reports `ChatEvent::RetryRequested`, which re-runs the
  completion.
- **Client-side decorations.** The window is created from
  `TitleBar::window_options()` and `KiiChat::render` emits `TitleBar` as the
  first child. Without it the title bar disappears and the window cannot be
  dragged.
- **Layout guards.** The `Chat` host needs `.flex_1().min_h_0()`; text that
  shrinks needs `.min_w_0()`.

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
cargo test                  # unit tests (API URL/SSE parsing)
```

Config lives at `%APPDATA%\KiiChat\config.json` (Windows),
`~/.config/KiiChat/config.json` (Linux), `~/Library/Application Support/KiiChat/config.json`
(macOS). Deleting it resets the app.

## Verifying a change

GPUI has no headless test harness for real frames here, so verify against the
running app. Release builds cannot be verified by reasoning; launch it.

1. **Launch and read the accessibility tree** (Windows):

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/uia.ps1
   ```

   It prints control type, accessible name and geometry for every element, which
   is enough to confirm panes, buttons, composer state and message rows exist.

2. **Drive it without a keyboard.** `scripts/invoke.ps1 -Name <label>` invokes a
   named button through UI Automation (use `-NameB64` for Chinese labels to
   dodge console encoding). Use it to exercise flows such as Retry.

3. **Exercise the network path against a fake provider**:

   ```sh
   python scripts/mock_openai.py      # serves /v1/models and a streaming completion on 127.0.0.1:18080
   ```

   Point a provider's Base URL at `http://127.0.0.1:18080/v1`, then check the
   server log for the request and `%APPDATA%\KiiChat\config.json` for the
   persisted streamed answer — that is proof the whole path ran.

4. **Screenshots** (visual confirmation):

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/capture.ps1
   ```

Hard rules for verification:

- Never claim a UI change works from a clean compile alone.
- Never leave `target/kiichat.exe` locked: stop the running app before rebuilding.
- The app writes to the real user config; if you seed it for a test, say so and
  restore or delete it afterwards.

## Repository hygiene

- `target/` must never be tracked (`git ls-files | grep -c '^target/'` is 0).
- Keep commits scoped and written as `type: summary` (`feat:`, `fix:`, `docs:`,
  `chore:`).
- Keep README's feature list and the GitHub description/topics in sync with what
  the app actually does.