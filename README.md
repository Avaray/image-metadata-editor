# 🧬 Image Metadata Editor `ime`

**ime** is a fast, lightweight, and portable [CLI](https://en.wikipedia.org/wiki/Command-line_interface) written in [Rust](https://rust-lang.org/) for **reading**, **writing**, and completely **wiping** metadata from image files.

## Installation

### Using Cargo (Recommended for Rust users)

If you have Rust installed, you can easily install the latest version directly from crates.io:

```bash
cargo install ime
```

### Pre-built binaries

Alternatively, you can download a pre-built executable for your operating system from the [Releases](https://github.com/Avaray/image-metadata-editor/releases/latest) page.

## Usage

```
ime <file> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--output <path>` | `-o` | Write result (or modified file) to `<path>` instead of stdout / in-place |
| `--strip` | `-s` | Remove all metadata from the file (JPEG & PNG only) |
| `--tui` | `-t` | Launch the Interactive Terminal UI (TUI) Mode |
| `--set Key=Value` | | Inject a metadata tag; repeatable (JPEG & PNG only) |
| `--key <path>` | `-k` | Extract a single value using a dot-notation path |
| `--dir` | `-d` | Explicitly indicate the input is a directory (optional) |
| `--recursive` | `-r` | Recursively process subdirectories when a directory is provided |
| `--version` | `-v` | Print the semver version number and exit |
| `--help` | `-h` | Print help and exit |

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

## Stripping metadata (`-s` / `--strip`)

Removes all EXIF and embedded metadata from the file in-place.  
Use `-o` to write to a new file instead (original is left untouched).

Supported formats: **JPEG**, **PNG**.

```bash
# In-place
ime photo.jpg -s

# Write to a new file
ime photo.jpg -s -o clean.jpg
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

## Interactive TUI Mode

`ime` includes a built-in Interactive Terminal UI (TUI) for browsing and editing metadata visually without leaving your terminal.

```bash
ime ./photos --tui
```

**Features:**
- **Split-screen layout**: Browse files on the left, view their live metadata on the right.
- **Vim-like navigation**: Use `Up`/`Down` or `j`/`k` to navigate, and `Tab` to switch focus between the file list and the metadata list.
- **Direct Editing**: Press `e` while focusing a metadata tag to edit its value directly in a popup. 
- **Bulk Strip**: Press `s` to quickly strip all metadata from the highlighted file.
- **Safety first**: If you press `q` (or `Esc`) to exit while you have unsaved edits in memory, `ime` will prompt you to save your changes (`nano`-style).

## Batch Processing

If you provide a directory path instead of a file, `ime` will automatically process all supported files in that directory. Use the `-r` flag to process subdirectories recursively.

```bash
# Strip metadata from all images in the 'photos' directory and its subdirectories
ime ./photos -s -r
```

## Supported formats

| Category | Formats | Read | Write |
|----------|---------|------|-------|
| Image | JPEG, PNG | ✅ | ✅ |
| Image | WebP | ✅ | ✅ |
| Image | HEIC/HEIF, AVIF, TIFF, CR3, RAF, IIQ | ✅ | ❌ |
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

This project is licensed under the [CC-BY-NC-4.0](LICENSE) License.
