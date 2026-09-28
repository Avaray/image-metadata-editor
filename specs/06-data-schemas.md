# Data Schemas and Edge Cases Specification

## Output / `--set` JSON shape

Metadata is always represented as one flat JSON object with two optional top-level sections:

```json
{
  "exif": { "Make": "Canon", "Model": "EOS R5" },
  "custom": { "rating": 5, "project": "summer-2026" }
}
```

- `exif` holds only **recognized standard EXIF tag names** (the canonical variant names from `little_exif`'s `ExifTag`, e.g. `Make`, `Model`, `DateTimeOriginal`, `ImageDescription`, `Artist`, `Copyright`, `GPSLatitude`). An unrecognized key here is a runtime error (exit `1`).
- `custom` holds anything else: arbitrary user-defined keys with arbitrary JSON values (objects, arrays, numbers, strings, booleans).
- `UserComment` (EXIF tag `0x9286`) is **reserved** and never appears under `exif`: it is the on-disk carrier for the `custom` section on JPEG/WebP (see below). A `--set` input containing `"exif": {"UserComment": ...}` is a runtime error. Users who want free-text notes put them under `custom`.
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

- **PNG**: the `custom` section is serialized as compact JSON, UTF-8 encoded, then hex-encoded, and stored in a single `tEXt` chunk with a fixed keyword (e.g. `ime:custom`). This is written/read/removed by hand (chunk length, type, CRC32 via the `crc` crate) — separate from `little_exif`'s own EXIF-carrying `zTXt` chunk.
- **JPEG / WebP**: the `custom` section is serialized as compact JSON and stored as the payload of the EXIF `UserComment` tag, using the standard 8-byte `"ASCII\0\0\0"` character-code prefix (JSON is ASCII-safe once non-ASCII characters are `\uXXXX`-escaped, so no other character-code prefix is needed).
- On `--wipe`, both the standard-EXIF storage and the custom-key storage described above are removed.
- **Determinism**: before serializing, the `custom` object's keys (at every nesting level) are sorted alphabetically. The same logical `--set` result must always produce byte-identical on-disk bytes, regardless of the order keys were supplied in, or any internal iteration order. This guarantee covers the `custom` section specifically (the part this project serializes itself); the `exif` section's on-disk tag ordering is whatever `little_exif` itself produces.

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
