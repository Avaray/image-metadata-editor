# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0] - 2026-10-05

### Added
- **Interactive Terminal User Interface (TUI)** for visual metadata browsing and editing (`--tui` flag or running `ime` with no arguments).
  - Dual-panel layout (Files and Metadata) with fast keyword search (`/`).
  - Native editors for deep JSON structures, multiline strings, and standard values.
  - In-memory explicit save model (`Ctrl+S`) allowing safe, atomic modifications before committing to disk.
  - `--expand` / `-x` flag for inline branch previews (showing object keys and array previews without drilling in).
  - Fast background file scanning using extension-sniffing for instant rendering, followed by magic-byte resolution.
  - Smart history path tracking: `Ctrl+Left` to instantly jump back to root, and `Ctrl+Right` to re-trace the deep path you came from.
- **Batch Processing & Directory Scanning**: Stage-3 architecture enabling `--set` and `--wipe` operations across entire directories (with `--recursive` support).
- **Dry Run Mode**: Added `--dry-run` to preview the resulting metadata JSON of `--set` and `--wipe` actions without writing changes to disk.
- **Native Custom Metadata Storage**: Replaced the previous monolithic `ime:custom` block with transparent mapping to native image storage. Custom metadata now natively writes to `zTXt`/`iTXt` chunks (PNG) and standard markers (JPEG/WebP).
- **JSON5 Fallback**: Embedded JSON strings (such as ComfyUI `workflow` payloads) containing non-standard tokens like `NaN` are now parsed seamlessly.

### Changed
- Total internal repository restructure and refactor transitioning from v1 to v2.
- Updated documentation and README to reflect the TUI and extended batch capabilities.
- Removed arbitrary extensions blocking; format detection relies purely on binary magic signatures (preventing mismatched file extensions from breaking edits).

## [1.6.0] - 2026-09-30

_Changes prior to 2.0.0 are excluded from this log._
