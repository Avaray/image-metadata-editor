# Future Ideas

## Native GUI (The Workspace Approach)

We recently implemented a built-in Interactive Terminal UI (TUI) via the `--tui` flag using `ratatui`. This allowed us to add a rich visual experience while keeping the single binary extremely lightweight, as TUI libraries avoid heavy graphics driver dependencies (`wgpu`, `vulkan`, etc.).

However, if we ever want to add a fully-fledged **Native Graphical User Interface** (e.g., using `egui` and `eframe`), the best architectural approach remains a **Cargo Workspace**.

### Structure
The project would be divided into three separate crates sharing the same root directory:
1. **`ime-core` (Library Crate)**: Contains 100% of the core logic (metadata parsing, reading, writing, wiping). It has zero CLI/GUI dependencies.
2. **`ime-cli` (Binary Crate)**: The current command-line and TUI interface. Depends on `ime-core`, `lexopt`, and `ratatui`.
3. **`ime-gui` (Binary Crate)**: The native windowed application. Depends on `ime-core` and `egui`.

### Directory Layout
```text
image-metadata-editor/
├── Cargo.toml (Workspace definition)
├── core/
│   ├── Cargo.toml
│   └── src/lib.rs
├── cli/
│   ├── Cargo.toml
│   └── src/main.rs
└── gui/
    ├── Cargo.toml
    └── src/main.rs
```

## TUI Improvements

While the current TUI is fully functional, here are ideas for future enhancements:

- **Mouse Support**: Allow clicking on the file list to switch files, or clicking on a metadata row to start editing it instantly. This was temporarily deferred to keep the initial TUI scope small, as managing scroll offsets manually for click targeting in `ratatui` requires a bit of state management overhead.
- **Search & Filter**: Add a search bar (`/`) to quickly filter the metadata list for specific keys or values (especially useful for images with hundreds of EXIF tags).
- **Batch Editing**: Allow selecting multiple files in the file tree (e.g., with `Space`) and injecting/stripping metadata from all of them at once directly from the TUI.
