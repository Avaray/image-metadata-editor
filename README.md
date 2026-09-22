# 🧬 Image Metadata Editor `ime`

**ime** is a fast, lightweight, and portable [CLI](https://en.wikipedia.org/wiki/Command-line_interface) written in [Rust](https://rust-lang.org/) for **reading**, **writing**, and completely **wiping** metadata from image files.

## Installation

Download a pre-built binary from the [Releases](../../releases) page.

## Usage

```
ime <file> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--output <path>` | `-o` | Write result (or modified file) to `<path>` instead of stdout / in-place |
| `--strip` | `-s` | Remove all metadata from the file (JPEG & PNG only) |
| `--set Key=Value` | | Inject a metadata tag; repeatable (JPEG & PNG only) |
| `--key <path>` | `-k` | Extract a single value using a dot-notation path |
| `--version` | `-v` | Print the semver version number and exit |
| `--help` | `-h` | Print help and exit |

---

## Reading metadata

Outputs all extracted metadata as a flat JSON object (one key per metadata directory).

```bash
# Print all metadata to stdout
ime photo.jpg

# Save to a file
ime photo.jpg -o meta.json
```

### Output example

```json
{
  "Exif": {
    "DateTimeOriginal": "2023-08-14 10:20:30",
    "FocalLength": "24/1 (24.0000)",
    "ISOSpeedRatings": "400"
  },
  "Tiff": {
    "Make": "SONY",
    "Model": "DSC-RX100M5A"
  },
  "PngText": {
    "parameters": "beautiful landscape ...",
    "workflow": "{\"nodes\": [...]}"
  }
}
```

---

## Extracting a single value (`-k` / `--key`)

Use dot-notation (same style as `jq`) to extract a single tag from the metadata.

- Leading `.` is optional — both `.Tiff.Make` and `Tiff.Make` work.
- Array indices are supported: `nodes[0].type`.
- **Smart JSON traversal:** if a tag value is itself a JSON string (common in ComfyUI `workflow` or `prompt` keys), `ime` automatically parses it and continues traversal.

```bash
# Extract a scalar value (printed as raw text, no quotes)
ime photo.jpg -k Tiff.Make
# Kyocera Visual Phone 

# Same with leading dot
ime photo.jpg -k .Tiff.Model
# VP-210

# Extract the raw generation parameters from a ForgeUI/A1111 PNG
ime image.png -k PngText.parameters

# Enter a ComfyUI workflow JSON embedded inside a tEXt chunk
ime image.png -k PngText.workflow.nodes[0].class_type

# Missing key → exit code 1
ime photo.jpg -k Exif.NonExistent
# Error: Key 'Exif.NonExistent' not found in metadata
```

When the result is a plain string it is printed without surrounding quotes.  
When the result is an object or array it is printed as pretty-printed JSON.

---

## Stripping metadata (`-s` / `--strip`)

Removes all EXIF and embedded metadata from the file in-place.  
Use `-o` to write to a new file instead (original is left untouched).

Supported formats: **JPEG**, **PNG**.

```bash
ime photo.jpg -s              # in-place
ime photo.jpg -s -o clean.jpg # write to a new file
```

## Injecting metadata (`--set`)

Sets one or more metadata tags. Existing tags that are not mentioned are preserved.  
Can be combined with `-o` to write to a new file.

Supported formats: **JPEG**, **PNG**.

```bash
# Standard EXIF tag
ime photo.jpg --set "ImageDescription=Sunset at the lake"

# Multiple tags
ime photo.jpg --set "Artist=Jan Kowalski" --set "Copyright=2025 Jan Kowalski"

# Write to a new file
ime photo.jpg --set "Software=ime" -o tagged.jpg

# Custom key (PNG → tEXt chunk; JPEG → UserComment JSON)
ime image.png --set "prompt=a cat sitting on a roof" --set "negative_prompt=blurry"

# Inject multiple tags from a JSON object
ime image.png --set-json '{"prompt":"a cat","steps":30,"cfg":7}'
```

## Supported formats

| Category | Formats | Read | Write |
|----------|---------|------|-------|
| Image | JPEG, PNG | ✅ | ✅ |
| Image | WebP, HEIC/HEIF, AVIF, TIFF, CR3, RAF, IIQ | ✅ | ❌ |
| Video | MP4, MOV, 3GP, MKV, WebM | ✅ | ❌ |

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Runtime error (I/O error, unsupported format, key not found, corrupt file) |
| `2` | Usage error (bad flags, missing file argument) |

## 🧾 Changelog

All notable changes to this project will be documented in the [CHANGELOG.md](CHANGELOG.md) file.

## 📄 License

This project is licensed under the [CC-BY-NC-4.0 License](LICENSE).
