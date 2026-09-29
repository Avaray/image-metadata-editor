use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=Cargo.lock");
    println!("cargo::rerun-if-changed=build.rs");

    // The exact compiler version, for the TUI About overlay.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = Command::new(rustc).arg("--version").output().ok().and_then(|out| String::from_utf8(out.stdout).ok()).map(|text| text.trim().to_string()).filter(|text| !text.is_empty()).unwrap_or_else(|| "unknown".to_string());
    println!("cargo::rustc-env=IME_RUSTC_VERSION={version}");

    // The exact pinned ratatui version, parsed out of Cargo.lock so the About
    // overlay can never drift out of sync with what is actually compiled in.
    let ratatui = std::fs::read_to_string("Cargo.lock").ok().and_then(|lock| locked_version(&lock, "ratatui")).unwrap_or_else(|| "unknown".to_string());
    println!("cargo::rustc-env=IME_RATATUI_VERSION={ratatui}");
}

/// Read the `version` of the `name` package from lockfile text: the first
/// `version = "..."` line following a `name = "<name>"` line.
fn locked_version(lock: &str, name: &str) -> Option<String> {
    let mut armed = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == format!("name = \"{name}\"") {
            armed = true;
        } else if armed && line.starts_with("version = \"") {
            return line.strip_prefix("version = \"")?.strip_suffix('"').map(str::to_string);
        } else if line == "[[package]]" {
            armed = false;
        }
    }
    None
}
