# Testing Strategy Specification

- All tests live in `tests/`, as integration tests that execute the compiled binary — not unit tests of internal functions.
- `assert_cmd` runs the binary; `predicates` asserts on stdout/stderr/exit code; `tempfile` provides disposable per-test directories and file copies so no test ever mutates a fixture in place.
- Fixtures: a small real PNG, JPEG, and WebP file per case needed below; one corrupt/truncated file; one non-image file (for format-detection rejection); one directory containing a symlink cycle (for the `--recursive` loop-handling test).

Minimum required coverage:

1. **Read** — `ime <file>` on each of PNG/JPEG/WebP prints the `exif`/`custom` JSON shape from `06-data-schemas.md` to stdout, exit `0`.
2. **Format detection by magic bytes, not extension** — a file with a misleading/renamed extension is still read/written correctly; a non-image file is rejected with exit `1`.
3. **`--set` merge** — setting one key leaves all other existing `exif`/`custom` keys unchanged (deep merge); setting a key to `null`/`""`/`{}`/`[]` deletes it; setting a whole section to `null` clears only that section.
4. **`--set` input forms** — inline JSON5 string, `-` (stdin), and `@<path>` (file) all produce the same result for equivalent payloads; unquoted keys, single quotes, and trailing commas are all accepted; `<file>` and `--set` both being `-` is a usage error (exit `2`).
5. **`--set` rejects unknown `exif` tag names and `"UserComment"` under `exif`** — exit `1`.
6. **`--wipe`** — after wiping, re-reading the file returns empty `exif`/`custom` sections, and the image-data chunks/segments (PNG `IDAT`; JPEG scan data; WebP `VP8`/`VP8L`/`VP8X`/`ALPH`) are byte-identical to the pre-wipe file — not a whole-file hash, since container bytes may legitimately change.
7. **`--output`, including `-` (stdout)** — writing to a different path (or stdout) leaves the original file byte-for-byte untouched; the new path/stream contains the modified result.
8. **Write atomicity/safety** — no partial/corrupt file is ever left at the destination path if the process is interrupted mid-write (simulate via a forced failure path if feasible) or given invalid input; post-write verification (see `03-business-logic.md`) does not false-positive on any successful `--set`/`--wipe`.
9. **Determinism** — running the same `--set` command twice, from two identical copies of the source file, produces byte-identical output files.
10. **`--dry-run`** — with `--set`/`--wipe`, prints the would-be resulting metadata to stdout and leaves the file completely unmodified (compare the file's bytes before and after).
11. **Batch/`--recursive`** — reading a directory prints the aggregated `{path: metadata}` object; a single bad file inside the batch is reported on stderr and doesn't stop the rest of the batch, the overall exit code is `1` if any file failed; a symlink cycle in the tree is reported as a per-file error rather than hanging the process.
12. **Exit code matrix** — spot-check that success is always `0`, runtime errors are always `1`, and usage errors (bad flags, `--output` with `--recursive`, double stdin) are always `2`.
