# TUI Specification

- The TUI is built using `ratatui` (v0.30) and `crossterm` (v0.29), with system clipboard integration via `arboard`.
- The top bar displays the current absolute path or directory.
- The bottom bar acts as a keybind legend explaining active shortcuts.
- The middle area is split vertically in a 35/65% proportion: the left panel contains the file explorer, and the right panel displays the metadata JSON tree.
- The file explorer always includes a `..` parent directory entry at the top, and preserves navigation state (remembering previously visited subdirectories and cursor positions).
- On Windows, the root directory lists logical drives with folder icons, whereas Unix systems start at `/`.
- The metadata panel allows exploring JSON branches interactively, remembering expanded paths upon re-entry.

> This file is intentionally left thin for now. Keybindings, focus flow, edit/confirm UX, and error/empty states are not yet specified — see the open questions in `00-index.md`. Do not expand this file without new instructions.
