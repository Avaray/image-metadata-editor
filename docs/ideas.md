# Future Ideas

## Adding a GUI (The Workspace Approach)

If we ever want to add a graphical user interface (e.g., using `egui` and `eframe`) without bloating the CLI version of the tool, the best architectural approach in Rust is a **Cargo Workspace**.

### Structure
The project would be divided into three separate crates sharing the same root directory:
1. **`ime-core` (Library Crate)**: Contains 100% of the core logic (metadata parsing, reading, writing, wiping). It has zero CLI parsing and zero GUI dependencies.
2. **`ime` (Binary Crate)**: The command-line interface. Depends on `ime-core` and `lexopt`. It parses terminal arguments and calls the core.
3. **`ime-gui` (Binary Crate)**: The graphical application. Depends on `ime-core` and `egui`. It handles the windowing, event loop, and calls the core.

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

### Benefits
- **Total Isolation**: The CLI crate doesn't have any dependencies on heavy graphics libraries like `winit` or `wgpu`. Its compiled binary size remains tiny (1-3MB).
- **Distribution**: We can publish `ime` and `ime-gui` as separate packages on `crates.io`.
- **User Experience**: 
  - Users typing `cargo install ime` get exactly what they want—a fast CLI.
  - Users who want the visual editor run `cargo install ime-gui`.
  - GitHub Releases would simply offer two different executables for standard users (e.g., `ime.exe` and `ime-gui.exe`).
