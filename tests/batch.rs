mod common;

use std::path::{Path, PathBuf};

use predicates::prelude::*;
use serde_json::{Value, json};

use common::{fixture, read_json, run_ime};

/// Build a temp work dir from `(source_fixture, relative_dest)` pairs,
/// creating parent directories as needed. Returns the temp dir (kept alive)
/// and its path.
fn work_dir(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    for (src, dest) in files {
        let path = dir.path().join(dest);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::copy(fixture(src), &path).unwrap();
    }
    let root = dir.path().to_path_buf();
    (dir, root)
}

/// Read a directory through the binary, asserting success and valid JSON.
fn read_batch(dir: &Path, extra: &[&str]) -> Value {
    let mut cmd = run_ime();
    cmd.arg(dir);
    for arg in extra {
        cmd.arg(arg);
    }
    let assert = cmd.assert().success();
    let output = assert.get_output();
    assert!(output.stderr.is_empty(), "stderr must be empty on success: {:?}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON")
}

fn key(dir: &Path, name: &str) -> String {
    dir.join(name).display().to_string()
}

#[test]
fn batch_read_prints_aggregated_object_without_descending() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.jpg", "b.jpg"), ("photo.webp", "sub/c.webp")]);
    let value = read_batch(&root, &[]);
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 2, "only top-level images, no subdir descent");
    for name in ["a.png", "b.jpg"] {
        let entry = &obj[&key(&root, name)];
        assert_eq!(entry["exif"]["Make"], Value::String("TestMake".to_string()), "{name}");
        assert_eq!(entry["custom"]["project"], Value::String("stage1".to_string()), "{name}");
    }
}

#[test]
fn batch_read_recursive_descends_into_subdirectories() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.webp", "sub/deep/c.webp")]);
    let value = read_batch(&root, &["--recursive"]);
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 2);
    assert_eq!(obj[&key(&root, "sub/deep/c.webp")]["exif"]["Make"], Value::String("TestMake".to_string()));
}

#[test]
fn batch_read_skips_unsupported_files_silently() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("not_an_image.txt", "note.txt"), ("not_an_image.txt", "blob.dat")]);
    let value = read_batch(&root, &["--recursive"]);
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 1, "only the image may appear");
    assert!(obj.contains_key(&key(&root, "a.png")));
}

#[test]
fn batch_read_reports_bad_file_and_continues() {
    let (_dir, root) = work_dir(&[("photo.png", "good.png"), ("corrupt.jpg", "bad.jpg")]);
    let assert = run_ime().arg(&root).assert().failure().code(1);
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bad.jpg"), "stderr must name the bad file: {stderr}");
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout must still be valid JSON");
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 1, "good file still reported");
    assert_eq!(obj[&key(&root, "good.png")]["exif"]["Make"], Value::String("TestMake".to_string()));
}

#[test]
fn batch_read_uses_magic_bytes_not_extension() {
    let (_dir, root) = work_dir(&[("photo.png", "weird.dat"), ("photo.jpg", "other.png")]);
    let value = read_batch(&root, &[]);
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 2, "misleading extensions must still be processed");
}

#[test]
fn batch_read_empty_directory_prints_empty_object() {
    let dir = tempfile::tempdir().unwrap();
    let value = read_batch(dir.path(), &["--recursive"]);
    assert_eq!(value, Value::Object(Default::default()));
}

#[test]
#[cfg(unix)]
fn recursive_follows_symlinked_directories() {
    let (_dir, root) = work_dir(&[("photo.png", "real/a.png")]);
    std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
    let value = read_batch(&root, &["--recursive"]);
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 2, "both the real and the symlinked path resolve");
    assert!(obj.contains_key(&key(&root, "real/a.png")));
    assert!(obj.contains_key(&key(&root, "linked/a.png")));
}

#[test]
#[cfg(unix)]
fn recursive_symlink_cycle_is_a_per_file_error_not_a_hang() {
    let (_dir, root) = work_dir(&[("photo.png", "sub/good.png")]);
    std::os::unix::fs::symlink(&root, root.join("sub/loop")).unwrap();
    let assert = run_ime().arg(&root).arg("--recursive").assert().failure().code(1);
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("loop"), "stderr must report the cyclic entry: {stderr}");
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout must still be valid JSON");
    assert_eq!(value.as_object().unwrap().len(), 1, "good file still reported");
}

#[test]
fn batch_set_updates_every_file_with_progress_and_summary() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.jpg", "b.jpg"), ("not_an_image.txt", "skip.txt")]);
    let assert = run_ime().arg(&root).arg("--set").arg(r#"{"custom": {"batch": true}}"#).assert().success().code(0).stdout(predicate::str::is_empty());
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "two progress lines plus summary: {stderr}");
    assert!(lines[0].starts_with("[1/2] OK ") && lines[0].contains("a.png"), "first line: {}", lines[0]);
    assert!(lines[1].starts_with("[2/2] OK ") && lines[1].contains("b.jpg"), "second line: {}", lines[1]);
    assert_eq!(lines[2], "Done: 2/2 succeeded, 0 errors");
    for name in ["a.png", "b.jpg"] {
        assert_eq!(read_json(&root.join(name))["custom"]["batch"], json!(true), "{name} must be updated in place");
    }
}

#[test]
fn batch_set_reports_bad_file_and_continues() {
    let (_dir, root) = work_dir(&[("photo.png", "good.png"), ("corrupt.jpg", "bad.jpg")]);
    let assert = run_ime().arg(&root).arg("--set").arg(r#"{"custom": {"k": 1}}"#).assert().failure().code(1).stdout(predicate::str::is_empty());
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{stderr}");
    assert!(lines[0].starts_with("[1/2] ERROR ") && lines[0].contains("bad.jpg"), "first line: {}", lines[0]);
    assert!(lines[1].starts_with("[2/2] OK ") && lines[1].contains("good.png"), "second line: {}", lines[1]);
    assert_eq!(lines[2], "Done: 1/2 succeeded, 1 errors");
    assert_eq!(read_json(&root.join("good.png"))["custom"]["k"], json!(1), "good file must still be updated");
}

#[test]
fn batch_set_recursive_with_invalid_payload_touches_nothing() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.jpg", "sub/b.jpg")]);
    let before_a = std::fs::read(root.join("a.png")).unwrap();
    let before_b = std::fs::read(root.join("sub/b.jpg")).unwrap();
    run_ime().arg(&root).arg("--recursive").arg("--set").arg("{oops").assert().failure().code(1).stdout(predicate::str::is_empty());
    assert_eq!(std::fs::read(root.join("a.png")).unwrap(), before_a);
    assert_eq!(std::fs::read(root.join("sub/b.jpg")).unwrap(), before_b);
}

#[test]
fn batch_wipe_empties_every_file() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.webp", "b.webp")]);
    run_ime().arg(&root).arg("--wipe").assert().success().stdout(predicate::str::is_empty()).stderr(predicate::str::contains("Done: 2/2 succeeded, 0 errors"));
    for name in ["a.png", "b.webp"] {
        assert_eq!(read_json(&root.join(name)), Value::Object(Default::default()), "{name} must be wiped");
    }
}

#[test]
fn batch_dry_run_prints_would_be_results_without_touching_disk() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.jpg", "b.jpg")]);
    let before: Vec<Vec<u8>> = ["a.png", "b.jpg"].iter().map(|name| std::fs::read(root.join(name)).unwrap()).collect();
    let mut cmd = run_ime();
    let assert = cmd.arg(&root).arg("--set").arg(r#"{"exif": {"Make": "Preview"}}"#).arg("--dry-run").assert().success();
    let output = assert.get_output();
    assert!(output.stderr.is_empty(), "stderr must be empty: {:?}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[key(&root, "a.png")]["exif"]["Make"], Value::String("Preview".to_string()));
    assert_eq!(value[key(&root, "b.jpg")]["exif"]["Make"], Value::String("Preview".to_string()));
    for (name, bytes) in ["a.png", "b.jpg"].iter().zip(before.iter()) {
        assert_eq!(&std::fs::read(root.join(name)).unwrap(), bytes, "{name} must be byte-identical");
    }
}

#[test]
fn dry_run_single_file_leaves_everything_untouched() {
    let (dir, root) = work_dir(&[("photo.jpg", "photo.jpg")]);
    let path = root.join("photo.jpg");
    let before = std::fs::read(&path).unwrap();
    let assert = run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Ghost"}, "custom": {"rating": null}}"#).arg("--dry-run").assert().success();
    let output = assert.get_output();
    assert!(output.stderr.is_empty(), "stderr must be empty: {:?}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["exif"]["Make"], Value::String("Ghost".to_string()));
    assert!(!value["custom"].as_object().unwrap().contains_key("rating"), "deleted key must be gone from the preview");
    assert_eq!(value["custom"]["project"], Value::String("stage1".to_string()), "untouched keys stay in the preview");
    assert_eq!(std::fs::read(&path).unwrap(), before, "file must be byte-identical");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp files may be left behind");

    let assert = run_ime().arg(&path).arg("--wipe").arg("--dry-run").assert().success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(value, Value::Object(Default::default()), "wiped preview must be empty");
    assert_eq!(std::fs::read(&path).unwrap(), before, "file must still be byte-identical");
}

#[test]
fn set_stdin_and_file_payloads_match_inline() {
    for (name, key) in [("photo.png", "via_stdin"), ("photo.jpg", "via_stdin"), ("photo.webp", "via_stdin")] {
        let (_dir, root) = work_dir(&[(name, "img")]);
        let path = root.join("img");
        run_ime().arg(&path).arg("--set").arg("-").write_stdin(format!(r#"{{"custom": {{"{key}": true}}}}"#)).assert().success();
        assert_eq!(read_json(&path)["custom"][key], json!(true), "{name}: --set - must apply");
    }

    let (_dir, root) = work_dir(&[("photo.png", "img.png")]);
    let payload = root.join("tags.json");
    std::fs::write(&payload, "{custom: {from_file: 7,},}").unwrap();
    let at = format!("@{}", payload.display());
    run_ime().arg(root.join("img.png")).arg("--set").arg(&at).assert().success();
    assert_eq!(read_json(&root.join("img.png"))["custom"]["from_file"], json!(7), "JSON5 from @file must apply");

    // `--set -` payloads also work in batch mode: one payload, every file.
    let (_dir, root) = work_dir(&[("photo.png", "a.png"), ("photo.jpg", "b.jpg")]);
    run_ime().arg(&root).arg("--set").arg("-").write_stdin(r#"{"custom": {"shared": 1}}"#).assert().success();
    for name in ["a.png", "b.jpg"] {
        assert_eq!(read_json(&root.join(name))["custom"]["shared"], json!(1), "{name}");
    }
}

#[test]
fn set_payload_file_errors_are_runtime_errors() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png")]);
    run_ime().arg(root.join("a.png")).arg("--set").arg("@does-not-exist.json").assert().failure().code(1).stdout(predicate::str::is_empty());
}

#[test]
fn stdin_write_without_output_goes_to_stdout() {
    let (_dir, root) = work_dir(&[("photo.png", "in.png")]);
    let input = std::fs::read(root.join("in.png")).unwrap();
    let assert = run_ime().arg("-").arg("--set").arg(r#"{"custom": {"piped": true}}"#).write_stdin(input).assert().success();
    let stdout = assert.get_output().stdout.clone();
    assert!(stdout.starts_with(&[0x89, 0x50, 0x4E, 0x47]), "stdout must be PNG image bytes");
    let roundtrip = root.join("roundtrip.png");
    std::fs::write(&roundtrip, &stdout).unwrap();
    assert_eq!(read_json(&roundtrip)["custom"]["piped"], json!(true));
    assert!(!root.join("-").exists(), "no file named '-' may be created");
    assert_eq!(std::fs::read(root.join("in.png")).unwrap(), std::fs::read(fixture("photo.png")).unwrap(), "no input file to modify; fixtures stay pristine");
}

#[test]
fn usage_errors_for_stage3_combinations() {
    let (_dir, root) = work_dir(&[("photo.png", "a.png")]);
    let file = root.join("a.png");
    // Double stdin: <file> - together with --set -.
    run_ime().arg("-").arg("--set").arg("-").write_stdin("{}").assert().failure().code(2);
    // Two --set - occurrences.
    run_ime().arg(&file).arg("--set").arg("-").arg("--set").arg("-").write_stdin("{}").assert().failure().code(2);
    // --output with --recursive (even on a single file) or with a directory.
    run_ime().arg(&file).arg("--set").arg("{}").arg("--output").arg("x.png").arg("--recursive").assert().failure().code(2);
    run_ime().arg(&root).arg("--set").arg("{}").arg("--output").arg("x.png").assert().failure().code(2);
    // Stdin <file> with --recursive.
    run_ime().arg("-").arg("--recursive").write_stdin(Vec::new()).assert().failure().code(2);
    // --dry-run without --set/--wipe, and together with --output.
    run_ime().arg(&file).arg("--dry-run").assert().failure().code(2);
    run_ime().arg(&file).arg("--set").arg("{}").arg("--dry-run").arg("--output").arg("x.png").assert().failure().code(2);
    // --recursive on a single file is accepted (a quiet no-op).
    run_ime().arg(&file).arg("--recursive").assert().success();
}
