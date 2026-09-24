# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.5.0] - 2026-09-24

### Added
- **CLI**: Allow launching TUI mode without a file argument (`-t` / `--tui` now defaults to the current directory).

### Fixed
- **TUI**: Strip Windows verbatim path prefixes (`\\?\`) to restore proper "go up" directory navigation.
- **TUI**: Rename virtual root label to "Devices & Drives" for a consistent cross-platform display.

### Changed
- **CI**: Add native macOS build targets for both `arm64` (Apple Silicon) and `x86_64` (Intel).
- **CI**: Add ARM Linux targets (`arm64`, `armv7`) and rework the manual publication workflow.
- **CI**: Add caching for `pip` packages in ARM build jobs.
- **CI**: Add manual binary selection inputs to trigger builds for specific platforms in `release.yml`.

## [1.4.0] - 2026-09-24

### Added
- **Explorer Mode**: Added a new file explorer mode to the TUI with Nerd Font icons for seamless directory navigation.
- **Power User Mode (`-p` / `--power`)**: Unlocks advanced/dangerous operations in both CLI and TUI.
- **Delete Files**: Added `--delete` flag (CLI) and `d` key shortcut (TUI) to delete files (requires Power User mode).
- **Clipboard Copy**: Press `c` in the TUI to copy the selected metadata value to the clipboard.
- **TUI Enhancements**: Added file filtering, multiline text pasting, and the ability to press `Enter` on an empty metadata list to add a new tag.
- **WebP Injection**: Added support for injecting metadata into simple WebP files.
- **Read-Only Indicators**: The TUI now supports more read-only formats and visually indicates them.

### Fixed
- Resolved numerous navigation and UI rendering bugs in the new explorer mode (e.g., drive root detection, virtual root handling).
- Improved error handling when saving files in the TUI and refined the force-quit logic.
- Fixed NPM package distribution issues by syncing `optionalDependencies` versions and hardening `bin.js` resolution.
- Hardened CI release scripts and enabled auto-triggering of releases on `Cargo.toml` version bumps.

## [1.3.0] - 2026-09-23

### Added
- **Add New Tag dialog (`e` on empty list)**: Press `e` in the Metadata panel when no items are visible (empty metadata, after stripping, or all results filtered out) to open a two-field dialog for adding a new tag. Tab / Enter switches between the Tag Name and Value fields.
- **`Ctrl+F` to activate search**: Alternative shortcut for `/` to enter search/filter mode.
- **`🧬` emoji** in the version label at the bottom-right corner.

### Fixed
- **NaN/Infinity in ComfyUI JSON**: Some PNG files saved by ComfyUI contain `"is_changed": NaN` — a legal JavaScript value but invalid JSON. `serde_json` rejected it, preventing drill-down into `PngText.prompt` keys. A new `sanitize_json()` state-machine replaces bare `NaN`, `Infinity`, and `-Infinity` with `null` before parsing, without touching values inside strings.
- **`Ctrl+/` shortcut removed**: `Ctrl+/` is not reliably delivered by most terminal emulators. Replaced with `Esc` (in Normal mode, clears the active filter before falling back to backing out of JSON levels).

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
