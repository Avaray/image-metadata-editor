# CLI Interface Specification

- Running the application with no arguments (`ime`) automatically launches the interactive TUI mode in the current directory.
- Running `ime <file>` prints all extracted metadata as JSON to stdout. See `06-data-schemas.md` for the exact shape.
- Running `ime <file> [OPTIONS]` allows querying or modifying a single file.
- `<file>` may be `-` to read the image from stdin instead of a path. Only valid in single-file mode (not a directory, not with `--recursive`).
- `--set <JSON>` / `-s`: Merges a JSON object into the file's metadata. Repeatable (each occurrence is merged in order, left to right). Default behavior is a **merge**: keys present in the input overwrite the corresponding existing value; anything not mentioned in the input is left untouched. See `03-business-logic.md` for full merge semantics and `06-data-schemas.md` for the JSON shape (`exif` / `custom` sections). The `<JSON>` value accepts three forms:
  - an inline string, in lenient JSON5-style syntax (unquoted keys, single quotes, trailing commas);
  - `-`, meaning read the entire payload from stdin (JSON5-style syntax also accepted). At most one `--set -` may appear per invocation, and it's a usage error if `<file>` is also `-` (stdin can't serve both at once).
  - `@<path>`, meaning read the entire payload from the named file (JSON5-style syntax also accepted).
- `--wipe` / `-w`: Removes all metadata possible from the file, in-place by default. See `03-business-logic.md` for exactly what counts as "possible to remove".
- `--dry-run`: Valid together with `--set` and/or `--wipe`. Computes the result in memory and prints the resulting metadata (same JSON shape as a normal read) to stdout, without writing anything to disk. Works in both single-file and directory/`--recursive` mode.
- `--output <path>` / `-o`: Valid together with `--set` and/or `--wipe` on a **single file** only (not with directories or `--recursive`). Writes the result to `<path>` instead of modifying the original in-place; the original file is left untouched. If `<path>` already exists, it is overwritten, silently, without confirmation (same behavior as `cp`). `<path>` may be `-` to write the resulting image bytes to stdout instead of a file. The output format always matches the input format (no conversion); a mismatched output extension is allowed but not recommended.
- `--recursive` / `-r`: Recursively processes subdirectories when a directory is provided. Follows symbolic links by default; a symlink loop is reported as a per-file error, not a hang (see `03-business-logic.md`). `--output` (a single destination, including its `-`/stdout form) and `<file> = -` are both inherently single-file, so neither is compatible with `--recursive`. `--set -` and `--set @<path>` work fine with `--recursive`: the same payload is merged into every matched file.
- `--tui` / `-t`: Opens the Interactive Terminal UI (TUI) for the given path.
- `--power` / `-p`: TUI power mode; skips confirmations and auto-saves on quit.
- `--version` / `-v`: Prints the semver version number and exits.
- `--help` / `-h`: Prints help and exits.

No other flags exist. In particular, there is no key-path / dot-notation addressing of any kind (no `--key`, no `--delete`): all reads return the full metadata object, and all writes go through `--set` with a JSON object (deleting a key is done by setting it to `null` — see `03-business-logic.md`).

Format detection (PNG vs. JPEG vs. WebP vs. unsupported) is always done by magic bytes, never by file extension. See `03-business-logic.md`.

## Examples

Strip only GPS data, keep everything else:
```
ime photo.jpg --set '{"exif":{"GPSLatitude":null,"GPSLongitude":null,"GPSLatitudeRef":null,"GPSLongitudeRef":null}}'
```

Tag an image piped in from `curl`, without touching disk for the input, using a JSON file for the payload (since stdin is already used for the image):
```
curl -s https://example.com/photo.jpg | ime - --set @tags.json > tagged.jpg
```

# Exit Codes
- Exit code `0` indicates success.
- Exit code `1` indicates a runtime error (I/O error, unsupported/undetected format, corrupt file, invalid `--set` JSON, unknown EXIF tag name in the `exif` section, post-write verification failure).
- Exit code `2` indicates a usage error (bad flags, missing file argument, incompatible flag combination such as `--output` with `--recursive`, or both `<file>` and `--set` set to `-` at once).
