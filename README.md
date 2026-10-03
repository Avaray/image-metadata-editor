# 🧬 IME

**IME** (Image Metadata Editor) is a fast, lightweight, and portable [CLI](https://en.wikipedia.org/wiki/Command-line_interface) written in [Rust](https://www.rust-lang.org/) for **reading**, **writing**, and **wiping** [metadata](https://en.wikipedia.org/wiki/Metadata) in `.png`, `.jpg`, and `.webp` files, with [JSON](https://en.wikipedia.org/wiki/JSON) input and output. It includes a built-in interactive terminal UI (TUI) for browsing and editing metadata visually.

Zero loss in quality: `ime` edits only metadata and never touches your pixels.

## Installation

### Pre-built binaries

Download a pre-built executable for your operating system from the [Releases](https://github.com/Avaray/image-metadata-editor/releases/latest) page.

Currently supported platforms are **Linux** (x86_64, ARM64, and ARMv7), **macOS** (Intel and Apple Silicon), and **Windows** (x86_64).

### Using Cargo

If you work in the [Rust ecosystem](https://www.rust-lang.org/), you can install the latest version directly from [crates.io](https://crates.io/crates/ime) using [cargo](https://doc.rust-lang.org/stable/cargo/).

```bash
cargo install ime
```

### Using JavaScript package managers

If you work in the [JavaScript ecosystem](https://stateofjs.com/en-US), you are probably familiar with package managers such as [NPM](https://docs.npmjs.com/downloading-and-installing-packages-globally), [PNPM](https://pnpm.io/global-packages), [Bun](https://bun.com/docs/pm/cli/install#global-packages), and others. You can use them to install `@avaray/ime` globally or run it without installing it. The installed executable is always called `ime`.

```bash
# Install globally using NPM
npm install -g @avaray/ime

# Run instantly without installation using Bun
bunx @avaray/ime photo.jpg
```

## Usage

```
ime <file> [OPTIONS]         Read or modify a single file (use - for stdin)
ime <dir> [--recursive]      Process every supported image in a directory
ime [--tui] [<path>]         Open the interactive TUI (defaults to .)
```

| Flag | Short | Description |
|------|-------|-------------|
| `--set <JSON>` | `-s` | Merge a JSON object into the metadata; repeatable |
| `--wipe` | `-w` | Remove all metadata from the file |
| `--dry-run` | | Print the would-be result without writing anything |
| `--output <path>` | `-o` | Write the result to `<path>` instead of in-place (`-` for stdout) |
| `--recursive` | `-r` | Recursively process subdirectories when a directory is provided |
| `--tui` | `-t` | Open the interactive terminal UI for the given path |
| `--power` | `-p` | TUI power mode: skip confirmations and auto-save on quit |
| `--watch` | | TUI-only: live-refresh the file list as files change |
| `--version` | `-v` | Print the semver version number and exit |
| `--help` | `-h` | Print help and exit |

### Reading metadata

Prints all extracted metadata as pretty-printed JSON with two sections: `exif` for standard EXIF tags and `custom` for user-defined keys.

```bash
ime photo.jpg
```

```json
{
  "custom": {
    "UserComment": {
      "project": "stage1",
      "rating": 5
    }
  },
  "exif": {
    "Make": "TestMake",
    "Model": "TestModel X1",
    "ExposureTime": "1/200",
    "GPSLatitude": ["52/1", "13/1", "0/1"]
  }
}
```

The `custom` shape is format-dependent: JPEG/WebP expose the single `UserComment` slot, while PNG exposes every native text chunk under `PngText.<keyword>` (e.g. `custom.PngText.workflow` for a ComfyUI file).

Values keep their native EXIF shape: rationals stay as `"numerator/denominator"` strings, dates stay as `"YYYY:MM:DD HH:MM:SS"`, and GPS coordinates stay as degree/minute/second triplets. A string value that itself holds a JSON object or array (such as an embedded ComfyUI `workflow` document) is parsed and shown as nested JSON.

### Setting metadata

`--set` deep-merges a JSON object into the file's metadata: keys present in the input overwrite the corresponding value, and everything else is left untouched. Repeat the flag to apply several payloads in order.

```bash
# Tag a photo (JSON5 syntax: unquoted keys, single quotes, trailing commas)
ime photo.jpg --set "{exif: {Artist: 'Jan Kowalski'}, custom: {UserComment: {rating: 5}}}"

# Delete keys by setting them to null (or "", {}, [])
ime photo.jpg --set '{"exif": {"GPSLatitude": null, "GPSLongitude": null}}'

# On PNG, custom keys live under PngText (one native text chunk per keyword)
ime image.png --set '{"custom": {"PngText": {"comment": "vacation photo"}}}'

# Read the payload from stdin or from a file instead of inline
echo '{"custom": {"UserComment": {"batch": 7}}}' | ime photo.jpg --set -
ime photo.jpg --set @tags.json
```

### Wiping metadata

Removes every standard EXIF tag, every custom key, the ICC color profile, and any other removable metadata, in-place by default.

```bash
# Strip everything, keeping only the pixels
ime photo.jpg --wipe

# Write the cleaned copy elsewhere, leaving the original untouched
ime photo.jpg --wipe -o clean.jpg
```

### Previewing changes

`--dry-run` runs the full `--set`/`--wipe` logic in memory and prints the resulting metadata, without writing anything to disk.

```bash
ime photo.jpg --set '{"exif": {"Make": "Ghost"}}' --dry-run
```

### Piping

`<file>` may be `-` to read the image from stdin, and `--output -` writes the resulting image to stdout. With stdin input and no `--output`, the result goes to stdout by default.

```bash
curl -s https://example.com/photo.jpg | ime - --set @tags.json > tagged.jpg
```

### Batch processing

Pass a directory to process every supported image it contains. Reads print an aggregated `{ "<path>": <metadata> }` object; writes happen in-place with per-file progress on stderr. Use `--recursive` to descend into subdirectories (symbolic links are followed).

```bash
ime ./photos --set '{"exif": {"Copyright": "2026 Jane Doe"}}' --recursive
```

A single bad file never stops the batch: it is reported on stderr, the rest continue, and the exit code is `1` if anything failed.

### Interactive TUI

`ime` includes a built-in terminal UI for browsing directories and editing metadata visually.

```bash
ime                  # browse the current directory
ime ./photos --tui   # browse a directory
ime photo.jpg -t     # open with a file pre-selected
```

| Keys | Action |
|------|--------|
| `Up`/`Down`, `Left`/`Right` | Navigate the file tree / drill through metadata |
| `Tab` | Switch between the file and metadata panels |
| `Enter`, `e` | Edit a value / edit a subtree as JSON |
| `n` | New entry in the current branch |
| `d` | Delete entry (with confirmation) |
| `w` | Wipe the selected file (with confirmation) |
| `/` | Search the focused panel |
| `c`, `Ctrl+C` | Copy a value / copy a path |
| `r` | Re-scan the current directory |
| `F1` | About overlay |
| `q` | Quit |

### Supported formats

Only **PNG**, **JPEG**, and **WebP** are supported. Formats are detected by magic bytes, never by file extension — a PNG renamed to `.dat` still works, while any other file kind (including HEIC, AVIF, TIFF, RAW, and all video formats) is rejected and never written.

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Runtime error (I/O error, unsupported format, corrupt file, invalid `--set` JSON, unknown EXIF tag) |
| `2` | Usage error (bad flags, missing file argument, incompatible flag combination) |

### Notes

- `ime` was originally created as a dependency for my custom [ComfyUI](https://github.com/comfy-org/ComfyUI) node collection, which is not publicly available yet.
- I do not plan to expand metadata-writing support to additional file formats, especially video formats.

### Changelog

All notable changes to this project will be documented in the [CHANGELOG.md](CHANGELOG.md) file.

### License

This project is licensed under the [CC-BY-NC-4.0](LICENSE) License.
