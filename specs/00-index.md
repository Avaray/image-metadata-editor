# Index

This is the entry point for the `ime` (Image Metadata Editor) spec set. Read the files in this order:

1. `01-core-vision.md` — what the project is, scope boundaries, license.
2. `02-cli-interface.md` — flags, arguments, exit codes.
3. `03-business-logic.md` — behavior rules: merge semantics, wipe, format detection, batch processing.
4. `05-architecture.md` — toolchain, dependencies (`Cargo.toml` is the source of truth for exact versions), error handling, write safety.
5. `06-data-schemas.md` — the JSON shape read and written, storage encoding per format, edge cases.
6. `07-testing-strategy.md` — what `tests/` must cover.
7. `08-release-and-distribution.md` — the CI build matrix, versioning, and npm/crates.io publishing this project already has.
8. `04-tui-spec.md` — interactive mode: keybindings, panels, watch mode, cross-platform rendering.

## Conventions

- **MUST** — required behavior; a deviation is a bug.
- **SHOULD** — a default the implementer can override with a documented reason.
- Every code example and JSON example in these specs is normative unless marked "illustrative".
- If a file in this set contradicts another, the more specific file wins (e.g. `06-data-schemas.md` wins over `03-business-logic.md` on exact JSON shape).

## Glossary

| Term | Meaning |
|---|---|
| **standard tag** | A tag defined by the EXIF spec (e.g. `Make`, `DateTimeOriginal`) and exposed under the `exif` section. |
| **custom key** | Any user-defined key that is not a standard EXIF tag; stored under the `custom` section. |
| **wipe** | Removing all metadata (both `exif` and `custom`) from a file; see `06-data-schemas.md`. |
| **magic bytes** | The file signature used to identify PNG/JPEG/WebP, independent of file extension. |
| **in-place** | Writing the result back to the original file path (the default, unless `-o/--output` is given). |

## Open questions (not yet decided — see `04-tui-spec.md`)

- Cycling through multiple search matches (next/previous) — only "jump to first match" is currently defined.
- Whether the wipe/delete confirmation's "Cancel" was meant to behave differently from "No" (currently spec'd as equivalent).
- The save-on-quit behavior without `--power` is inferred from the CLI spec's "auto-saves on quit" wording, not explicitly given — confirm it matches intent.
