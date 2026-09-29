# TUI Specification

- The TUI is built using `ratatui` (v0.30) and `crossterm` (v0.29), with system clipboard integration via `arboard` and text-input via `tui-input`. See `05-architecture.md` for why.
- The top bar displays the current absolute path or directory, inside a border — the same border style as the file tree and metadata panels below it, so all four regions (top bar, file tree, metadata tree, bottom legend) read as one consistent bordered layout.
- The bottom bar acts as a keybind legend, also inside a border, and updates to reflect whichever panel/mode currently has focus (the legend for the file tree differs from the legend for the metadata tree or an open dialog). Format: `[key] Action`, pairs separated by ` | `, e.g. `[←/→/↑/↓] Navigate | [e/Enter] Edit | [c] Copy | [d] Delete`. Only show keys that are actually valid in the current context.
- The middle area is split vertically in a 35/65% proportion: the left panel contains the file explorer, and the right panel displays the metadata JSON tree.
- On Windows, the root directory lists logical drives with folder icons, whereas Unix systems start at `/`.
- All UI copy (messages, prompts, labels) is in English, regardless of the terminal's locale.

## File tree panel

- Always includes a `..` parent directory entry at the top (except at the filesystem root), and preserves navigation state per directory (remembering previously visited subdirectories and cursor positions).
- Only directories and files detected as PNG/JPEG/WebP by magic bytes (see `03-business-logic.md`) are listed — anything else is omitted from the list entirely, not just unstyled. This scan happens once per directory (on entry, on `r`, or on a watch-triggered refresh — see below), not on every keystroke.
- Icons: a folder icon for directories, an image icon for PNG/JPEG/WebP files. Use plain Unicode symbols, not Nerd Font glyphs — Nerd Fonts require a patched font the user may not have installed, which would render as missing-glyph boxes and undermine the cross-platform/WSL2 consistency required below.
- Navigation:
  - **Up/Down** — move the cursor among the currently visible entries. Wraps around: Up from the first entry moves to the last, Down from the last moves to the first.
  - **PageUp/PageDown** — move the cursor by one panel's height. Clamps at the ends (stops on the first/last entry) rather than wrapping.
  - **Right** — on a directory, enters it. On a supported file, switches focus to the metadata panel (its preview is already loading/loaded — see Loading states below) rather than doing nothing. No-op specifically on the `..` entry (going up is Left's job — see below; entering `..` via Right would be a redundant, confusing second way to do the same thing).
  - **Left** — go to the parent directory, regardless of which entry the cursor is currently on (not only when the cursor is on `..`).
- **`c`** — copies the selected entry's name (file or directory) as plain text.
- **Ctrl+C** — copies the selected entry's full absolute filesystem path.
- **`r`** — re-scans the current directory (same effect as a watch-triggered refresh; see Watch mode).
- Must stay responsive with directories containing thousands of entries: rendering draws only the visible slice of the list each frame (never the whole collection), so scroll/cursor movement cost doesn't grow with directory size.

## Metadata tree panel

- When no file is loaded: displays "Open an image to load metadata." instead of a tree.
- Header/label: shows **"Metadata"** while at the root (nothing drilled into). Once the user has drilled into a branch, the header instead shows the breadcrumb path to the current location, e.g. `PngText > comment > workflow > nodes`. An array index is appended to its parent segment in brackets rather than being its own segment, e.g. `PngText > comment > workflow > nodes[3]`.
- Navigation:
  - **Up/Down** — move the cursor among the currently visible rows. Wraps around: Up from the first row moves to the last, Down from the last moves to the first.
  - **PageUp/PageDown** — move the cursor by one panel's height. Clamps at the ends rather than wrapping.
  - **Right** — expand/drill into the selected branch (updates the breadcrumb).
  - **Left** — collapse/go back up one level (updates the breadcrumb back toward "Metadata"). If there's no further level to collapse (already at the root breadcrumb) — or the selected file has no metadata at all — Left instead switches focus to the file tree panel.
- Editing:
  - **Enter** — on an object/array node, behaves exactly like Right (drill in). On a scalar (leaf) node, opens the value editor for that leaf directly, since there's nowhere further to drill.
  - **`e`** — opens an editor for the selected node's entire subtree as raw JSON5 text, on *any* node (leaf or branch), scoped to only that node and its descendants — independent of how deep Enter/Right has drilled.
- **`n`** — creates a new entry inside the branch the user is currently drilled into (per the breadcrumb, not necessarily the cursor row). If that branch is an object, prompts for a key name then a value; if it's an array, prompts for a value only (appended at the end).
- **Leaf value parsing** (applies to both `n`'s value prompt and Enter-on-a-leaf editing, *not* to `e`'s subtree editor): if the typed text starts with `{` or `[`, it's parsed strictly as JSON5, same as `--set`, and a parse failure is a validation error as usual. Otherwise, JSON5 parsing is tried first (so bare literals like `true`, `false`, `null`, a number, or an explicitly quoted string all work); if that fails too, the raw typed text is taken as a plain string, as if it had been quoted. This means typing e.g. `asdf` as a value just works as the string `"asdf"` — the common case doesn't require the user to type quotes by hand.
- **`d`** — deletes the currently selected key/entry. Confirmation gate applies (see Confirmations below).
- **`c`** — copies the selected node's value: a scalar copies its raw value (unquoted for strings); an object/array copies its compact JSON, directly reusable as a `--set` payload elsewhere.
- **Ctrl+C** — copies the full path to the selected node in dot/bracket notation, e.g. `PngText.comment.workflow.nodes[3]`. This is a TUI-only convenience and does not reintroduce CLI dot-notation addressing (see `02-cli-interface.md`).
- Validation: every edit (`e`, Enter-on-leaf, `n`) is validated immediately, using the same rules as `--set` — including that a name added under `exif` must be a recognized standard tag (`06-data-schemas.md`). An invalid result is not committed: the offending node is visually highlighted (e.g. a distinct border/text color) and an inline message names the problem, until the user fixes or cancels the edit.
- Must stay responsive on large/deep trees (e.g. an embedded ComfyUI `workflow` with hundreds of nodes): only the visible slice of currently-expanded rows is rendered each frame.

## Global keys

- **Tab** — switches focus between the file tree panel and the metadata panel.
- **`/`** — opens a search prompt scoped to whichever panel currently has focus (file/directory names in the file tree; keys and values, case-insensitively, in the metadata tree). Enter jumps to and selects the first match; Esc cancels the search.
- **`w`** — wipes all metadata from the currently selected file (the same operation as CLI `--wipe`, run immediately once confirmed). Confirmation gate applies.
- **F1** — opens an "About" overlay: author, license, repository URL, and the exact Rust and `ratatui` versions the running binary was built against, all embedded at compile time from `Cargo.toml`/`Cargo.lock` so the panel can never drift out of sync with what's actually pinned.
- **Esc** — closes whatever overlay/prompt is currently open (search, About, a confirmation dialog, an in-progress edit) without applying it.

### Confirmations

`w` (wipe) and `d` (delete key) each show a **Yes / No** confirmation before acting, unless `--power`/`-p` is active, in which case the action runs immediately with no prompt. Escape is always equivalent to **No** — both dismiss with no effect. On **Yes**, the action writes to disk immediately; it is not deferred.

### Editing and saving

Value edits (`e`, Enter-on-a-leaf, `n`) are held in memory while the editor is open; confirming the edit (Enter) both validates and writes it to disk immediately, through the same atomic-write-and-verify pipeline `--set` uses (`05-architecture.md`). This is the same immediacy as `w`/`d` — there is no separate "unsaved changes" buffer spanning multiple edits.

> **Assumption:** `02-cli-interface.md`'s existing `--power` bullet ("skips confirmations and auto-saves on quit") implies something to save on quit in non-`--power` mode. Since every edit above already writes on confirmation, this should be rare in practice; this spec treats it as covering the edge case of a confirmed-but-not-yet-flushed write. Confirm this matches the intent, or say what "auto-saves on quit" was meant to cover instead.

## Text editing

Every inline text input (the leaf value editor, the `e` subtree editor, the search prompt, and the key/value prompts for `n`) supports the same cursor behavior: Left/Right/Home/End/Backspace/Delete, plus word-boundary jumps via **Ctrl+Left/Right** (and **Alt+Left/Right** as an alias), matching conventions from `nano` and most terminal editors. See `05-architecture.md` for the widget this is built on.

## Watch mode (`--watch`)

- At most one filesystem watch is ever active at a time: the currently displayed directory. Changing directories (including going up) drops the previous watch and starts a new one for the new directory; quitting the TUI drops it. There is no "watch the whole tree" mode and no scenario where more than one watch, or a watch outliving the process, can exist.
- Not recursive — only the currently displayed directory is watched, not its subdirectories.
- A filesystem change event triggers the same refresh as `r`, debounced (coalescing a rapid burst of events, e.g. many files being written at once, into a single refresh) rather than re-scanning per event.
- On WSL2, watching a Windows-mounted path (anything under `/mnt/*` via DrvFs) is known to be unreliable for changes made from the Windows side — a long-standing WSL/DrvFs limitation this app cannot fix. Changes made from inside WSL itself, and watching native WSL2 filesystem paths (e.g. under `/home/...`), work reliably. This must degrade gracefully (silently fall back on the user's manual `r`), never hang or error out.
- See `05-architecture.md` for how this is kept leak-free.

## Loading states

Reading a directory (a magic-byte scan over potentially thousands of files) or loading a file's metadata can take longer than the UI should ever block for. Both run off the main thread, so the render loop stays responsive throughout. Two different navigation actions trigger this, and they're handled differently:

- **Changing directory (Left/Right in the file tree)** — while the scan for the target directory is in flight, further Left/Right presses in the file tree are ignored: there's nothing to navigate to yet, and letting them through risks a second (or third) directory scan starting before the first one even finishes. Up/Down, Tab, and the other global keys stay responsive throughout. **Esc** gives up waiting on an in-flight scan and unblocks navigation immediately; the abandoned scan is left to finish on its own in the background rather than force-killed, and its result is simply discarded when it arrives (see `05-architecture.md`).
- **Metadata preview (Up/Down moving the file-tree cursor onto a different file)** — not blocked. Moving the cursor immediately starts loading the newly selected file's metadata; any still-in-flight load for the previous selection becomes irrelevant, and only the result for whichever file is currently selected when it arrives is ever shown.
- In both cases, a result is only applied to the UI if it's still what the user is currently looking at — a stale result (superseded by newer navigation before it arrived) is silently discarded rather than applied. This is what makes rapid navigation safe even while an earlier request is still resolving.
- If a load takes longer than **1 second**, show a loading indicator in place of the panel's content — a simple ASCII spinner cycling `| / - \`, not a Unicode glyph, for the same cross-platform-rendering reasons as above. Nothing is shown for loads that resolve within that first second, so the common case never flickers.

See `05-architecture.md` for how this is implemented.

## Performance

Both panels must stay responsive on directories with thousands of files and metadata trees with hundreds of nested nodes. This is a hard requirement, not an optimization: see the rendering and scanning rules under each panel above, and `05-architecture.md`.

## Cross-platform rendering

The TUI must render identically — no stray blank lines, no unexpected scrollback growth — on Windows Terminal, native Linux terminals, macOS Terminal/iTerm2, and WSL2 running under Windows Terminal. See `05-architecture.md` for the implementation requirements this depends on, and `07-testing-strategy.md` for the manual verification it needs.

## Open questions

See `00-index.md`.
