mod common;

use std::time::Duration;

use predicates::prelude::*;

use common::{fixture, run_ime};

// The interactive TUI cannot run headless, so these tests cover what can be
// observed without a terminal: flag validation, path handling, and clean
// failure when no TTY is present (exit 1, never a hang).

#[test]
fn watch_and_power_require_tui_mode() {
    let file = fixture("photo.png");
    run_ime().arg(&file).arg("--watch").assert().failure().code(2).stderr(predicate::str::contains("--watch requires TUI mode"));
    run_ime().arg(&file).arg("--power").assert().failure().code(2).stderr(predicate::str::contains("--power requires TUI mode"));
    run_ime().arg(&file).arg("-p").assert().failure().code(2);
}

#[test]
fn tui_rejects_cli_write_flags() {
    let dir = tempfile::tempdir().unwrap();
    run_ime().arg("--tui").arg(dir.path()).arg("--set").arg("{}").assert().failure().code(2);
    run_ime().arg("--tui").arg(dir.path()).arg("--wipe").assert().failure().code(2);
    run_ime().arg("--tui").arg(dir.path()).arg("--set").arg("{}").arg("--dry-run").assert().failure().code(2);
    run_ime().arg("--tui").arg(dir.path()).arg("--set").arg("{}").arg("--output").arg("x.png").assert().failure().code(2);
    run_ime().arg("--tui").arg(dir.path()).arg("--recursive").assert().failure().code(2);
    run_ime().arg("--tui").arg("-").assert().failure().code(2);
}

#[test]
fn tui_rejects_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    run_ime().arg("--tui").arg(dir.path().join("does-not-exist")).assert().failure().code(1).stdout(predicate::str::is_empty());
}

#[test]
fn tui_without_tty_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixture("photo.png"), dir.path().join("a.png")).unwrap();
    // Piped stdio is not a terminal: exit 1 with a message, never a hang.
    // The timeout is load-bearing, not a backstop: raw mode alone cannot
    // detect this (crossterm enables it via /dev/tty, which exists under
    // `cargo test` in any real terminal), so without an explicit stdio gate
    // the TUI starts drawing into the pipe and this test hangs instead.
    run_ime().arg("--tui").arg(dir.path()).timeout(Duration::from_secs(10)).assert().failure().code(1).stderr(predicate::str::contains("cannot start TUI"));
    run_ime().arg("-t").arg(dir.path().join("a.png")).timeout(Duration::from_secs(10)).assert().failure().code(1).stderr(predicate::str::contains("cannot start TUI"));
    // A bare launch is also TUI mode, so it fails the same way headless.
    run_ime().current_dir(dir.path()).timeout(Duration::from_secs(10)).assert().failure().code(1).stderr(predicate::str::contains("cannot start TUI"));
}

#[test]
fn help_mentions_tui_flags() {
    run_ime().arg("--help").assert().success().stdout(predicate::str::contains("--tui")).stdout(predicate::str::contains("--power")).stdout(predicate::str::contains("--watch")).stdout(predicate::str::contains("--expand"));
}
