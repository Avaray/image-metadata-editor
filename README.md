# mex

**mex** (Metadata EXtractor) is a blazingly fast, lightweight, and portable CLI written in **Rust** that reads and writes metadata from image files.

It was primarily designed to handle, strip, and inject metadata into AI-generated images (e.g., from **ComfyUI**, **ForgeUI**, or Stable Diffusion), making it easy to extract or manipulate custom generation prompts and workflow parameters.

- Zero runtime dependencies — single statically linked binary
- Blazingly fast: reads only the bytes that hold metadata; run time does not depend on file size
- Outputs readable text or JSON
- Can strip all metadata or inject custom key/value pairs (JPEG & PNG)

---

## Scope & Limitations

- **Primary Target:** Modification features (`--strip`, `--set`) strictly target **JPEG** and **PNG** files, as these are the standard for AI image generation.
- **Read-Only Video/Audio:** While `mex` can extract and read metadata from various other formats (like MP4, MOV, WebP, TIFF), this is strictly a **read-only** feature. Support for modifying video, audio, or other exotic image formats is minimal and is **not planned** for future expansion.
- **Custom Tags:** Full custom tag injection (e.g., `prompt`, `workflow`) is natively supported via `tEXt` chunks in PNG. For JPEG, custom tags are packed into the standard `UserComment` EXIF field.

---

## Installation

```bash
cargo build --release
# Binary: target/release/mex.exe (Windows) or target/release/mex (Linux/macOS)
```

---

## Usage

```
mex <file> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--json` | `-j` | Print metadata as JSON instead of plain text |
| `--output <path>` | `-o` | Write result (or modified file) to `<path>` instead of stdout / in-place |
| `--strip` | `-s` | Remove all metadata from the file (JPEG & PNG only) |
| `--set Key=Value` | | Inject a metadata tag (repeatable, JPEG & PNG only) |
| `--version` | `-v` | Print the semver version number and exit |
| `--help` | `-h` | Print help and exit |

---

## Reading metadata

```bash
# Print metadata as readable text (default)
mex photo.jpg

# Print metadata as JSON
mex photo.jpg --json

# Save output to a file
mex photo.jpg -j -o meta.json
```

### Text output example

```
File: /home/user/photo.jpg
[Exif]
  DateTimeOriginal: 2023-08-14 10:20:30
  FocalLength: 24/1 (24.0000)
  ISOSpeedRatings: 400
[Tiff]
  Make: SONY
  Model: DSC-RX100M5A
```

### JSON output example

```json
{
  "file": "/home/user/photo.jpg",
  "directories": {
    "Exif": {
      "DateTimeOriginal": "2023-08-14 10:20:30",
      "FocalLength": "24/1 (24.0000)",
      "ISOSpeedRatings": "400"
    },
    "Tiff": {
      "Make": "SONY",
      "Model": "DSC-RX100M5A"
    }
  }
}
```

---

## Stripping metadata

Removes all EXIF / metadata from the file. Operates in-place by default; use `-o` to write to a new file instead (leaving the original untouched).

Supported formats: **JPEG**, **PNG**.

```bash
# In-place strip
mex photo.jpg --strip
mex photo.jpg -s

# Write stripped copy to a new file
mex photo.jpg -s -o photo_clean.jpg
```

> [!NOTE]
> For video and audio formats, `--strip` returns an error. Extraction (`mex <file>`) still works for all supported formats.

---

## Injecting metadata (`--set`)

Sets one or more metadata tags. Can be used multiple times. Existing tags not mentioned in the command are preserved.

Supported formats: **JPEG**, **PNG**.

```bash
# Set a standard EXIF tag
mex photo.jpg --set "ImageDescription=Sunset at the lake"

# Set multiple tags at once
mex photo.jpg --set "Artist=Jan Kowalski" --set "Copyright=2025 Jan Kowalski"

# Write to a new file instead of modifying in-place
mex photo.jpg --set "Software=mex" -o photo_tagged.jpg

# Set a completely custom tag (non-EXIF key)
mex photo.png --set "prompt=a cat sitting on a roof" --set "negative_prompt=blurry"
```

### Supported standard EXIF keys

| Key | EXIF meaning |
|-----|-------------|
| `ImageDescription` | Caption / description |
| `Make` | Camera manufacturer |
| `Model` | Camera model |
| `Software` | Software used |
| `Artist` | Author / creator |
| `Copyright` | Copyright notice |
| `DateTimeOriginal` | Original capture date (`YYYY:MM:DD HH:MM:SS`) |
| `UserComment` | Free-form comment (raw bytes) |

### Custom (non-EXIF) keys

Any key not listed above is treated as a **custom tag**:

- **PNG**: Written as a native `tEXt` chunk — readable by any PNG-aware tool (e.g. Stable Diffusion viewers read `prompt` this way).
- **JPEG**: Packed as a JSON object into the `UserComment` EXIF field.

---

## Supported formats (read)

| Category | Formats |
|----------|---------|
| Image | JPEG, PNG, WebP, HEIC/HEIF, AVIF, TIFF, CR3, RAF, IIQ |
| Video | MP4, MOV, 3GP, MKV, WebM |
| Audio | (via container metadata where applicable) |

---

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success (even if no metadata was found) |
| `1` | Runtime error (I/O error, unsupported format, corrupt file) |
| `2` | Usage error (bad flags, missing file argument) |
