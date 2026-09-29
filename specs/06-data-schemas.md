# Data Schemas and Edge Cases Specification

## Output / `--set` JSON shape

Metadata is always represented as one flat JSON object with two optional top-level sections:

```json
{
  "exif": { "Make": "Canon", "Model": "EOS R5" },
  "custom": { "PngText": { "comment": "vacation photo", "workflow": { "nodes": [ "..." ] } } }
}
```

(The example above is PNG; see "Custom-key on-disk storage" below for why `custom`'s shape differs by format — `PngText` for PNG, `UserComment` for JPEG/WebP.)

- `exif` holds only **recognized standard EXIF tag names** (the canonical variant names from `little_exif`'s `ExifTag`, e.g. `Make`, `Model`, `DateTimeOriginal`, `ImageDescription`, `Artist`, `Copyright`, `GPSLatitude`). An unrecognized key here is a runtime error (exit `1`).
- `custom` holds anything else. Its exact shape is format-dependent — see "Custom-key on-disk storage" below — because it's a direct, transparent view of each format's own native text/comment storage, not a private `ime` format. This is what makes metadata written by other tools (e.g. ComfyUI or similar tools embedding a `workflow`/`prompt` payload as a PNG text chunk) show up correctly without `ime` having written it first.
- `UserComment` (EXIF tag `0x9286`) is **reserved** and never appears under `exif`: it is the on-disk carrier for JPEG/WebP's `custom.UserComment` (see below). A `--set` input containing `"exif": {"UserComment": ...}` is a runtime error. Users who want free-text notes put them under `custom`.
- Either section may be omitted from `--set` input (that section is left untouched) or from read output (that section is empty).
- This same shape is what `ime <file>` prints, and what `--set <JSON>` accepts (subject to the lenient JSON5 syntax below).

## Value formatting for `exif`

| EXIF value kind | JSON representation |
|---|---|
| String (`STRING`) | JSON string, as-is |
| Integer types | JSON number |
| Rational (e.g. `ExposureTime`) | JSON string `"<numerator>/<denominator>"` (avoids float rounding) |
| Date/time (e.g. `DateTimeOriginal`) | JSON string, kept in EXIF's native `"YYYY:MM:DD HH:MM:SS"` form — no timezone conversion is performed |
| GPS coordinates | JSON array of the underlying rational triplet, in EXIF's native degrees/minutes/seconds form — no conversion to decimal degrees |
| Binary / `UNDEFINED` type | Lower-case hex string (via the `hex` crate) |

## `--set` input syntax

- `--set <JSON>` / `-s` accepts a lenient JSON5-style syntax: unquoted keys, single-quoted strings, trailing commas (parsed via the `json5` crate, then converted to the canonical JSON value used for merging).
- See `03-business-logic.md` for the full merge algorithm (deep merge, empty-value deletion, section-level `null`).

## Custom-key on-disk storage

`custom` is a direct, transparent view of each format's own native text-comment storage — never a private `ime`-only blob. This is what makes pre-existing metadata from other tools (e.g. a `workflow`/`prompt` payload a generation tool embedded) show up correctly on read, without `ime` having written it first.

- **PNG**: every `tEXt`/`zTXt`/`iTXt` chunk in the file — except the one keyword `little_exif` itself reserves for carrying standard EXIF data (`"Raw profile type exif"`, already surfaced properly under `exif`) — is exposed under `custom.PngText.<keyword>`, where `<keyword>` is that chunk's own keyword exactly as it appears in the file (e.g. `custom.PngText.comment`, `custom.PngText.workflow`). Each chunk's text value goes through the same smart-JSON-traversal rule as everywhere else (`03-business-logic.md`): if it parses as JSON, it's embedded as nested JSON (this is how a `workflow` chunk's JSON text becomes real nested structure like `custom.PngText.workflow.nodes`); if it doesn't parse, it's kept as a plain string, never discarded or errored on. Writing `custom.PngText.<keyword>` creates or updates that exact chunk (as `iTXt`, to support UTF-8 without needing to hex-encode or otherwise transform the text); setting it to `null` removes that chunk. `custom.PngText` itself is the only key PNG's `custom` section can have — there's no separate free-form namespace, because PNG's own metadata model *is* "a set of independently-named text chunks", nothing more.
- **JPEG / WebP**: `custom.UserComment` is the only key JPEG/WebP's `custom` section can have (JPEG/WebP's EXIF data model has exactly one general-purpose free-text slot — `UserComment` — not PNG's open set of named chunks; see `05-architecture.md`). Reading: `UserComment`'s raw text goes through the same smart-JSON-traversal rule — if it's valid JSON (including something `ime` wrote itself), it becomes `custom.UserComment`'s structured value; if it's plain text written by another tool (a free-text comment, or any non-JSON convention), it's kept as `custom.UserComment`'s raw string value. Writing: if `custom.UserComment`'s value is a JSON object/array, it's compact-serialized into `UserComment`'s text; if it's already a plain string, that string is written as-is. Either way, the standard 8-byte `"ASCII\0\0\0"` character-code prefix is used (JSON is ASCII-safe once non-ASCII characters are `\uXXXX`-escaped; a plain string value must likewise be ASCII, or use `\uXXXX` escaping consistently with how it's read back).
- On `--wipe`, both the standard-EXIF storage and all custom-key storage described above are removed (every `tEXt`/`zTXt`/`iTXt` chunk for PNG; the `UserComment` tag for JPEG/WebP).
- **Determinism**: whenever multiple chunks/keys are written together in one operation, they're written in sorted (alphabetical, by keyword/key) order; whenever a value being serialized is itself a JSON object (a PNG chunk's JSON-parsed text, or `custom.UserComment`'s value), its keys are sorted alphabetically at every nesting level before serializing. The same logical `--set` result must always produce byte-identical on-disk bytes, regardless of the order keys were supplied in or any internal iteration order. This guarantee covers only what this project serializes itself; the `exif` section's on-disk tag ordering is whatever `little_exif` produces.

## Edge cases

- **Unsupported/undetected format** (magic bytes don't match PNG/JPEG/WebP): runtime error, exit `1`.
- **Corrupt file** (magic bytes match but the container can't be parsed): runtime error, exit `1`.
- **Unknown tag name under `exif`**, or `"UserComment"` under `exif`: runtime error, exit `1`.
- **Invalid `--set` JSON/JSON5**: runtime error, exit `1`.
- **`--wipe`**: removes everything possible — all `exif` tags, the entire `custom` section, ICC color profile, and any other ancillary/non-essential chunk or segment. Only the chunks/segments strictly required to decode the image are kept (PNG `IHDR`/`PLTE`/`IDAT`/`IEND`; JPEG scan data; WebP `VP8`/`VP8L`/`VP8X`/`ALPH`). Pixel data is always bit-identical before and after.
- **`--output` targeting an existing path**: the existing file is overwritten, silently, without confirmation (same behavior as `cp`).
- **`<file>` and `--set` both given as `-`**: usage error, exit `2` — stdin can't be consumed twice.
- **`<file>` or `--output` set to `-` combined with a directory or `--recursive`**: usage error, exit `2` — a single stream can't stand in for a set of files. `--set -` and `--set @<path>` are unaffected and work with `--recursive` (the same payload is merged into every matched file).
- **`--dry-run`**: never touches disk or stdout image bytes; prints the resulting `exif`/`custom` JSON only, exit `0`.
- **Post-write verification failure** (the written result's image-data bytes don't match the source): runtime error, exit `1`. This should not happen in normal operation; the destination (in-place path, `--output <path>`, or stdout) is left exactly as if the write had never been attempted.
- **Symlink cycle during `--recursive`**: reported as that single entry's error (skipped, not a hang); the rest of the batch continues, matching the general batch-error behavior in `03-business-logic.md`.
