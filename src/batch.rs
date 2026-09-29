use std::path::{Path, PathBuf};

use crate::format;

/// One collected batch entry. The collection is fully enumerated up front so
/// batch writes can print `[<n>/<total>]` progress with a known total, then
/// sorted by path for deterministic progress and output order.
pub enum Entry {
    Work { path: PathBuf },
    Failed { path: String, message: String },
}

impl Entry {
    /// The path as shown in progress lines, error messages, and the
    /// aggregated `{ "<path>": <metadata> }` output object: the walked path
    /// as-is (the directory argument joined with the relative path).
    pub fn display(&self) -> String {
        match self {
            Self::Work { path, .. } => path.display().to_string(),
            Self::Failed { path, .. } => path.clone(),
        }
    }
}

/// Enumerate every batch candidate under `root`: regular files whose magic
/// bytes match PNG/JPEG/WebP become `Work`; unreadable files and traversal
/// failures (e.g. a symlink cycle, which `walkdir` reports instead of
/// looping forever) become `Failed`. Anything else — directories,
/// unsupported files, non-file entries — is silently skipped, per
/// `03-business-logic.md`.
pub fn collect(root: &Path, recursive: bool) -> Vec<Entry> {
    let mut walker = walkdir::WalkDir::new(root).follow_links(true).min_depth(1);
    if !recursive {
        walker = walker.max_depth(1);
    }
    let mut entries = Vec::new();
    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                let path = err.path().unwrap_or(root).display().to_string();
                entries.push(Entry::Failed { path, message: err.to_string() });
                continue;
            }
        };
        let path = entry.path();
        let display = path.display().to_string();
        let kind = match std::fs::metadata(path) {
            Ok(kind) => kind,
            Err(err) => {
                entries.push(Entry::Failed { path: display.clone(), message: format!("cannot read '{display}': {err}") });
                continue;
            }
        };
        if kind.is_dir() || !kind.is_file() {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                entries.push(Entry::Failed { path: display.clone(), message: format!("cannot read '{display}': {err}") });
                continue;
            }
        };
        if format::detect(&bytes).is_some() {
            entries.push(Entry::Work { path: path.to_path_buf() });
        }
    }
    entries.sort_by(|a, b| a.display().cmp(&b.display()));
    entries
}
