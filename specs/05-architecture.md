# Architecture and Dependencies Specification

`Cargo.toml` is the source of truth for exact version numbers; this file explains *why* each dependency is there and any compatibility constraint that isn't obvious from the manifest alone.

## Toolchain

- Rust edition **2024**, minimum supported Rust version (MSRV) **1.88**.
- The MSRV is set by `ratatui` 0.30's own MSRV (1.88), which is higher than edition 2024's own minimum (1.85). If `ratatui` is ever downgraded, this MSRV can move down accordingly — but see the pinning note below before doing that.

## Dependencies and rationale

**CLI & filesystem**
- `lexopt` — minimal, dependency-free argument parsing.
- `walkdir` — directory recursion for `--recursive`, with `.follow_links(true)`. `walkdir` detects symlink cycles itself and yields an error for the offending entry instead of looping forever, so following symlinks by default (see `03-business-logic.md`) doesn't need any extra cycle-detection code.

**Metadata**
- `nom-exif` (`default-features = false`) — reads EXIF from PNG/JPEG/WebP. Default features pull in an optional async/tokio backend that `ime` never needs, since the whole tool is synchronous.
- `little_exif` — writes/removes standard EXIF tags across PNG/JPEG/WebP through one API (`Metadata::set_tag`, etc.). Two behaviors to design around:
  - On PNG, it stores EXIF in a `zTXt` chunk using the ImageMagick-style "raw profile type exif" convention — not a plain `tEXt` chunk. This is separate from, and must not be confused with, this project's own custom-key `tEXt` chunk (see `06-data-schemas.md`).
  - On WebP, writing metadata to a simple lossy (`VP8`) or lossless (`VP8L`) file promotes the container to extended format (`VP8X`), wrapping the original image chunk unchanged. This is expected: the pixel bitstream is preserved byte-for-byte, only the RIFF container gains a header and an `EXIF` chunk. Don't treat this promotion as a bug.
- `crc` and `hex` — for manual, low-level chunk work that `little_exif` doesn't expose: constructing/locating/removing PNG `tEXt` chunks for custom-key storage, and stripping ancillary chunks on `--wipe`. `crc` computes the chunk CRC32; `hex` encodes binary/JSON payloads into the ASCII-safe form PNG text chunks and EXIF `UNDEFINED`-type values require.
- `serde_json` — canonical JSON representation used for CLI output and for the `--set` merge target.
- `json5` — parses the lenient JSON5-style syntax accepted by `--set` (unquoted keys, single quotes, trailing commas), then converts into a `serde_json::Value` for merging. `serde_json` alone cannot parse this syntax.

**TUI**
- `ratatui` — terminal UI framework.
- `crossterm` — terminal backend.
- `arboard` (`default-features = false`) — clipboard integration. Default features pull in the `image` crate for clipboard image support, which is never needed since the TUI only ever copies text (a JSON value, a key path, a file path).
- `tui-input` (`default-features = false`, `features = ["ratatui-crossterm"]`) — single-line, cursor-editable text input, including built-in word-boundary movement (`GoToPrevWord`/`GoToNextWord`), used for every inline text field: the leaf value editor, the search prompt, and the `n` key/value prompts. Pinned versions matter here: `tui-input` 0.15's `ratatui-crossterm` feature requires `ratatui ^0.30.2`/`crossterm ^0.29.0`, exactly what's already pinned above — verified to resolve to one shared `ratatui`/`crossterm` each, no duplication.
  - **Not `tui-textarea`**: its latest release (0.7.0) pins `ratatui ^0.29.0`, incompatible with our `ratatui 0.30` — Cargo would have to compile two different `ratatui` versions side by side, and `tui-textarea`'s widget wouldn't type-check against our 0.30-based `Frame`/`Buffer` anyway. The `e` key's multi-line raw-JSON subtree editor should be built as a thin multi-line wrapper composed of several `tui-input` lines instead of pulling in a second, conflicting text-widget crate — this also keeps every text field in the app using one consistent editing primitive.
- `notify` — cross-platform filesystem watching for `--watch` (inotify on Linux, FSEvents on macOS, `ReadDirectoryChangesW` on Windows). Its `RecommendedWatcher` cleans itself up on `Drop`; the "at most one watch, never outliving the process" guarantee in `04-tui-spec.md` depends on the `Watcher` value's lifetime being tied directly to "the directory currently being displayed" — don't route its events through a detached thread or channel that could outlive the `Watcher` itself.

**Dev dependencies (see `07-testing-strategy.md`)**
- `assert_cmd`, `predicates`, `tempfile`.

## A version-pinning note

`ratatui` and `crossterm` must be kept in a pair where `ratatui` does not vendor a *second*, different `crossterm` version internally (this happened between `ratatui` 0.29 and an explicitly-pinned `crossterm` 0.29 — `ratatui` 0.29 depends on `crossterm` 0.28 internally, so both 0.28 and 0.29 end up in the dependency tree at once, which is fragile: event/backend types from one don't interoperate with the other). `ratatui` 0.30.x depends on `crossterm` 0.29 internally, so pinning `crossterm` to `0.29` alongside `ratatui = "0.30"` resolves to a single shared `crossterm` version. When bumping either crate in the future, verify with `cargo tree -i crossterm` that only one version resolves.

## Post-write verification, without a new dependency

The post-write verification described in `03-business-logic.md` does **not** require decoding pixels (and therefore no `image`-crate-style dependency). It only needs to prove the image-data chunks/segments are unchanged, and those are already identified for `--wipe`'s critical-chunk allowlist (PNG `IDAT`; JPEG scan data; WebP `VP8`/`VP8L`/`VP8X`/`ALPH`). Verification means: extract those same raw bytes from the source and from the write result, and compare — a direct byte comparison, or a `crc` checksum of them (reusing the dependency already present for PNG chunk work), is sufficient. This is the same low-level chunk access already needed for custom-key storage and `--wipe`, not a separate subsystem.

## No image re-encoding

None of the chosen crates decode or re-encode pixel data. Every metadata operation (read, write, wipe) manipulates container-level chunks/segments only (PNG chunks, JPEG APPn/COM segments, WebP RIFF chunks). Pixel bytes are always preserved bit-for-bit — this is a hard constraint, not an implementation detail.

## Error handling

- Use a single top-level error type (e.g. via `thiserror`, or a hand-written enum) that every fallible path converts into. Do not `panic!`/`unwrap()` on user-controlled input (file contents, `--set` JSON, CLI arguments).
- The error type must distinguish **usage errors** (bad flags/arguments — exit `2`) from **runtime errors** (bad file, bad data, I/O failure — exit `1`); see `02-cli-interface.md`.
- Error messages go to stderr; stdout is reserved for the JSON output of successful reads.

## Background loading, without an async runtime

Directory scans and per-file metadata loads (`04-tui-spec.md`'s "Loading states") each run on a plain `std::thread::spawn` worker that sends its one result back over a `std::sync::mpsc::channel`; the main thread polls that channel non-blockingly on each render iteration. This deliberately doesn't pull in an async runtime (`tokio` or similar) — it would be a heavy, single-purpose dependency for something `std::thread` + `std::sync::mpsc` already covers, and it stays consistent with `nom-exif` already being built with its optional async backend disabled, since the rest of the tool is synchronous by design.

- **Staleness, not cancellation.** A spawned worker is never force-killed. Each request is tagged with what it's for (the target directory path, or the file path being previewed); when a result arrives, the main thread applies it only if that tag still matches what the user is currently looking at, and discards it otherwise. An abandoned scan (e.g. after the user presses Esc to stop waiting on a slow network-mounted directory) is left to finish naturally; it's a short-lived, self-terminating thread that exits the moment it sends its one result, not a persistent process — a different, simpler concern than the `notify::Watcher` lifecycle in watch mode, which does need active teardown.
- **Animating the spinner without new input.** The main event loop uses `crossterm::event::poll(Duration)` with a short timeout (e.g. ~100ms) instead of blocking indefinitely on `read()`; a poll timeout with no event is treated as a tick that can advance the spinner frame and trigger a redraw, so the spinner keeps animating even while the user isn't pressing anything.
- Optionally, debounce metadata-preview loads by a few tens of milliseconds so holding Up/Down doesn't spawn a worker thread for every file flown past while key-repeat is active — a minor refinement, not a correctness requirement, since staleness-discarding already makes the unoptimized version correct either way.

## Cross-platform rendering (Windows Terminal, WSL2 included)

- Enter the alternate screen buffer (`EnterAlternateScreen`/`LeaveAlternateScreen`) and enable raw mode for the whole TUI session. Never let raw `println!`/`print!`/`eprintln!` output reach the terminal while the TUI is active, including from a panic — install a panic hook that restores the terminal (disable raw mode, leave the alternate screen) *before* printing anything, so a panic mid-render can't leave stray output sitting in the user's normal scrollback.
- Never rely on the terminal's own automatic line-wrapping: lay out and truncate/wrap all text within `ratatui`'s cell grid yourself (its default behavior when used as intended), rather than writing a line longer than the reported terminal width and letting the terminal wrap it.
- This specifically matters because of a currently-open ConPTY bug ([microsoft/terminal#16603](https://github.com/microsoft/terminal/issues/16603)) where line-wrapping under ConPTY — which is what WSL2 renders through, via Windows Terminal — can insert spurious blank lines into scrollback once a window narrower than the content triggers a wrap. The mitigation above (never depend on terminal-side wrapping) sidesteps this rather than working around ConPTY itself, which this project has no control over.
- Re-query terminal size on every `crossterm` resize event rather than caching it once at startup.
- This class of bug is terminal-emulator-specific and isn't caught by `ratatui`'s headless `TestBackend`; see `07-testing-strategy.md` for the manual verification it needs instead.

## Write safety

- All writes (`--set`, `--wipe`, in-place or via `--output`) are atomic: write the full result to a temp file in the same directory as the destination, then rename it over the destination. Never write partial data to the real destination path.
- If any step fails (parse, merge, encode), the original file must be left completely untouched.
- The resulting file (in-place or via `--output`) gets standard OS-default permissions from the process umask. `ime` does not copy the original file's permission bits, and does not set mtime back to the original value — both are left to whatever the OS does naturally when a new file is created and renamed into place.
- When the source is `-` (stdin) and/or the destination is `--output -` (stdout), there is no path to rename: the full input is read into memory, the full result is assembled in memory (including post-write verification against it), and only *then* is it written to stdout in one go. The ordering guarantee stays the same as the file case — verification happens before the result becomes visible to the outside world, it's just an in-memory buffer standing in for the temp file.
