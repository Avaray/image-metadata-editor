# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-09-22

### Added
- Complete CLI for reading, writing, and wiping metadata from image files.
- Command-line arguments handling (e.g., `-o/--output`, `-s/--strip`, `-k/--key`, `--set`).
- **Read functionality**: Output all extracted metadata as a flat JSON object grouped by metadata directory (Exif, Tiff, PngText).
- **Extract functionality**: Extract single scalar values or nested JSON values using dot-notation paths (similar to `jq`).
- **Strip functionality**: Completely strip all EXIF and embedded metadata from files (supports JPEG and PNG).
- **Write functionality**: Inject and modify metadata tags via the `--set` and `--set-json` flags (supports JPEG and PNG).
- Comprehensive metadata reading support for various image and video formats (JPEG, PNG, WebP, HEIC/HEIF, AVIF, TIFF, CR3, RAF, IIQ, MP4, MOV, 3GP, MKV, WebM).
