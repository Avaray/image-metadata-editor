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

## Write safety

- All writes (`--set`, `--wipe`, in-place or via `--output`) are atomic: write the full result to a temp file in the same directory as the destination, then rename it over the destination. Never write partial data to the real destination path.
- If any step fails (parse, merge, encode), the original file must be left completely untouched.
- The resulting file (in-place or via `--output`) gets standard OS-default permissions from the process umask. `ime` does not copy the original file's permission bits, and does not set mtime back to the original value — both are left to whatever the OS does naturally when a new file is created and renamed into place.
- When the source is `-` (stdin) and/or the destination is `--output -` (stdout), there is no path to rename: the full input is read into memory, the full result is assembled in memory (including post-write verification against it), and only *then* is it written to stdout in one go. The ordering guarantee stays the same as the file case — verification happens before the result becomes visible to the outside world, it's just an in-memory buffer standing in for the temp file.
