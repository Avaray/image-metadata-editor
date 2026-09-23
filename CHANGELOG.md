# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.0] - 2026-09-23

### Added
- **Search & Filter (`/`)**: Press `/` in the Metadata panel to enter search mode. Typing filters keys and values in real-time (case-insensitive). The panel title shows the active query and match count. Press `Enter` to confirm the filter and return to navigation, or `Esc` to clear the filter entirely.
- **NPM distribution**: Published as `@avaray/ime` on the NPM registry. Install globally with `npm install -g @avaray/ime` or run instantly via `npx @avaray/ime`.

## [1.1.0] - 2026-09-23

### Added
- **Interactive TUI Mode (`-t` / `--tui`)**: A fully featured split-screen terminal user interface for browsing files and live-editing metadata.
- **Nested JSON Drill-Down**: In TUI mode, pressing `Enter` on JSON values steps inside them, enabling direct metadata exploration and editing of nested properties.
- **WebP Support**: Added lightweight, zero-copy support for stripping and writing metadata to WebP images.
- **Batch Processing**: Added support for reading, writing, and stripping metadata from entire directories, with an optional `-r` / `--recursive` flag.

### Fixed
- Decoded hexadecimal strings for `Exif.UserComment` to properly display embedded Unicode/ASCII text.
- Fixed a bug on Windows where key events were triggered twice in the TUI (both on press and release).
- Improved parsing of stringified JSON blocks inside PNG chunks, allowing them to be correctly unescaped and drilled into within the TUI.
- Fixed visual glitches in the TUI by upgrading text inputs to handle block cursors.

## [1.0.0] - 2026-09-22

### Added
- Complete CLI for reading, writing, and wiping metadata from image files.
- Command-line arguments handling (e.g., `-o/--output`, `-s/--strip`, `-k/--key`, `--set`).
- **Read functionality**: Output all extracted metadata as a flat JSON object grouped by metadata directory (Exif, Tiff, PngText).
- **Extract functionality**: Extract single scalar values or nested JSON values using dot-notation paths (similar to `jq`).
- **Strip functionality**: Completely strip all EXIF and embedded metadata from files (supports JPEG and PNG).
- **Write functionality**: Inject and modify metadata tags via the `--set` and `--set-json` flags (supports JPEG and PNG).
- Comprehensive metadata reading support for various image and video formats (JPEG, PNG, WebP, HEIC/HEIF, AVIF, TIFF, CR3, RAF, IIQ, MP4, MOV, 3GP, MKV, WebM).
