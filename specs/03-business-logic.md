# Business Logic Specification

## Supported formats

- Read and write capability is supported **only** for PNG, JPEG, and WebP.
- Format is identified by **magic bytes**, never by file extension:
  - PNG: `89 50 4E 47 0D 0A 1A 0A`
  - JPEG: starts with `FF D8 FF`
  - WebP: bytes 0-3 are `RIFF`, bytes 8-11 are `WEBP`
- Any file whose magic bytes don't match one of the above is unsupported. In single-file mode this is a runtime error (exit `1`). In directory/`--recursive` mode, non-matching files are silently skipped (a directory naturally contains unrelated files; this is not an error).
- The file extension is never trusted for format decisions and is not required to be correct. When writing via `--output`, the tool writes bytes in the *input's* detected format regardless of the output path's extension (no format conversion is ever performed).

## `--set` merge semantics

- The merge is a **deep (recursive) merge**, not a shallow replace: only the keys explicitly present in the input JSON are changed; every other existing key, at every nesting level, is left exactly as it was.
- The input JSON has two optional top-level sections, `exif` and `custom` (see `06-data-schemas.md` for the full shape). Each section is merged independently.
- Setting a key's value to an empty value (`null`, `""`, `{}`, `[]`) deletes that entry.
- Setting an entire section to `null` (e.g. `{"exif": null}`) deletes everything in that section, leaving the other section untouched.
- Multiple `--set` occurrences are applied in sequence, each merging on top of the result of the previous one.
- A key under `exif` that is not a recognized standard EXIF tag name is a runtime error (exit `1`). Arbitrary keys are only accepted under `custom`.
- `--set` input (inline, `-`/stdin, or `@<path>`) is parsed once per occurrence, as lenient JSON5. See `02-cli-interface.md` for the three input forms.

## `--wipe`

- Removes **all** metadata possible from the file: every standard EXIF tag, every custom key, ICC color profile, and any other ancillary/non-essential chunk or segment the format allows removing.
- Never touches the chunks/segments required to decode the image itself (e.g. PNG `IHDR`/`PLTE`/`IDAT`/`IEND`, the JPEG scan data, the WebP `VP8`/`VP8L`/`VP8X`/`ALPH` image chunks). Pixel data is always preserved bit-for-bit; `ime` never re-encodes an image.
- `--wipe` and `--set` are mutually exclusive within a single invocation (wipe first, then set, doesn't apply — pick one).

## `--dry-run`

- Runs the full `--set`/`--wipe` merge logic in memory, then prints the resulting metadata (the same `exif`/`custom` JSON shape a normal read would produce) to stdout, exit `0`.
- Nothing is written to disk — no temp file, no rename, no stdout image bytes.
- In directory/`--recursive` mode, prints the same aggregated `{ "<path>": <metadata> }` shape that a batch read would.

## Custom keys and smart JSON traversal

- Custom keys are stored differently per format, as a direct view of that format's own native text storage (PNG's `tEXt`/`zTXt`/`iTXt` chunks; JPEG/WebP's `UserComment` tag) rather than a private `ime`-only format — see `06-data-schemas.md`. This is deliberate: it's what makes metadata another tool already embedded (e.g. a generation tool's `workflow`/`prompt` payload in a PNG chunk) show up correctly without `ime` ever having written it. Text that isn't valid JSON (written by some other, non-`ime` tool) is kept as a plain string rather than discarded or treated as an error.
- Whenever `custom` data is (re)serialized, keys are written in sorted (alphabetical) order at every level — this makes the same logical `--set` result produce a byte-identical output file every time, regardless of internal iteration order.
- When metadata is read and printed, any string value (in `exif` or `custom`) that is itself valid JSON is parsed and embedded as nested JSON rather than left as an escaped string, and this is applied recursively. This makes embedded documents — such as ComfyUI's `workflow`/`prompt` payloads — directly inspectable. This traversal is output-only; it never changes what's stored on disk.
- Full metadata is always printed as pretty-printed JSON (there is no separate raw-scalar output mode).

## Post-write verification

- After building the write result (in-place, `--output <path>`, or `--output -`) and before it becomes visible to the outside world (before the atomic rename, or before anything is written to stdout), `ime` re-derives a checksum of the image-data bytes (the same chunks/segments listed as untouchable under `--wipe`) from the result and compares it to the checksum taken from the original file before the operation started.
- A mismatch is an internal error (exit `1`): the destination is left as it was before the write (temp file discarded, nothing renamed into place; nothing written to stdout), exactly as any other failed write.
- This applies to every write operation (`--set` and `--wipe` alike), not just `--wipe`; there is no flag to disable it.

## Batch processing

- Given a directory (with `--recursive` to descend into subdirectories), reads print an aggregated `{ "<path>": <metadata> }` JSON object to stdout; unsupported files are omitted from this object.
- `--recursive` follows symbolic links by default. A symlink that creates a cycle is detected (not an infinite loop) and reported as that entry's per-file error; the rest of the batch continues.
- Given a directory with `--set`/`--wipe`, each supported file is modified in-place, with per-file progress written to stderr, one line per file, in the form:
  ```
  [<n>/<total>] OK <path>
  [<n>/<total>] ERROR <path>: <message>
  ```
  followed by a final summary line once the batch completes:
  ```
  Done: <succeeded>/<total> succeeded, <errors> errors
  ```
- A single file's failure during batch processing is reported on stderr (using the `ERROR` line above) and does not stop the batch. If any file failed, the process exits with code `1` after finishing the batch.
