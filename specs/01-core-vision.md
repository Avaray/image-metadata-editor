# Project Vision and Scope

* The `ime` (Image Metadata Editor) project is a fast, lightweight, and portable CLI tool written in Rust 1.88+ (2024 edition).
* The primary objective of the software is reading, writing, and wiping metadata in `.png`, `.jpg`, and `.webp` files. No other image or video format is read, written, or otherwise supported, in any capacity.
* The application uses JSON for input and output operations.
* It includes a built-in Interactive Terminal UI (TUI) for browsing and editing metadata visually.
* The release profile uses aggressive optimizations (`opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = true`) to ensure uncompromising performance and minimal binary size.
* License: **CC BY-NC 4.0** (Attribution-NonCommercial 4.0 International). See `Cargo.toml`'s `license` field (SPDX `CC-BY-NC-4.0`).

# Out of Scope
* Any file format other than PNG, JPEG, and WebP — including other image formats (e.g. HEIC, AVIF, TIFF, RAW) and all video formats — is explicitly out of scope. Such files are neither read nor written; see `03-business-logic.md` for how they're detected and rejected.
* Any form of image re-encoding, transcoding, or pixel modification. `ime` only ever touches metadata containers (EXIF segments/chunks, text chunks); pixel data is always preserved bit-for-bit.
