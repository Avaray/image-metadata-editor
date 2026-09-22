use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

fn get_fixtures() -> Vec<PathBuf> {
    let mut fixtures = Vec::new();
    if let Ok(entries) = fs::read_dir("tests/fixtures") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                fixtures.push(path);
            }
        }
    }
    fixtures
}

#[test]
fn test_cli_on_fixtures() {
    let fixtures = get_fixtures();
    if fixtures.is_empty() {
        println!("No fixtures found, skipping test.");
        return;
    }

    for path in fixtures {
        // Output is always JSON now
        let assert = Command::cargo_bin("ime")
            .unwrap()
            .arg(&path)
            .assert()
            .success();

        let json_out: serde_json::Value =
            serde_json::from_slice(&assert.get_output().stdout).unwrap();
        assert!(json_out.get("file").is_none());

        // Output file
        let temp_dir = tempfile::tempdir().unwrap();
        let out_file = temp_dir.path().join("out.json");
        Command::cargo_bin("ime")
            .unwrap()
            .arg(&path)
            .arg("-o")
            .arg(&out_file)
            .assert()
            .success()
            .stdout(predicate::str::is_empty()); // print nothing on success

        assert!(out_file.exists());
        let content = fs::read(&out_file).unwrap();
        let _json_file: serde_json::Value = serde_json::from_slice(&content).unwrap();
    }
}

#[test]
fn test_missing_file() {
    Command::cargo_bin("ime")
        .unwrap()
        .arg("does_not_exist.jpg")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Error:"));
}

#[test]
fn test_unsupported_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let unsupported = temp_dir.path().join("unsupported.txt");
    fs::write(&unsupported, b"not an image or video").unwrap();

    Command::cargo_bin("ime")
        .unwrap()
        .arg(&unsupported)
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_no_arguments() {
    Command::cargo_bin("ime")
        .unwrap()
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn test_help_version() {
    Command::cargo_bin("ime")
        .unwrap()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
    Command::cargo_bin("ime")
        .unwrap()
        .arg("-v")
        .assert()
        .success()
        // -v prints only the semver, e.g. "0.1.0\n"
        .stdout(predicate::str::is_match(r"^\d+\.\d+\.\d+\n$").unwrap());
}

#[test]
fn test_truncated_fixtures() {
    let fixtures = get_fixtures();
    if fixtures.is_empty() {
        return;
    }

    for path in fixtures {
        let content = std::fs::read(&path).unwrap();
        let mut sizes_to_test = vec![0, 1, 10, content.len() / 2, content.len().saturating_sub(1)];
        sizes_to_test.retain(|&s| s < content.len());

        let temp_dir = tempfile::tempdir().unwrap();
        for (i, &size) in sizes_to_test.iter().enumerate() {
            let trunc_path = temp_dir.path().join(format!("trunc_{}.bin", i));
            std::fs::write(&trunc_path, &content[..size]).unwrap();

            let assert = Command::cargo_bin("ime").unwrap().arg(&trunc_path).assert();
            let code = assert.get_output().status.code().unwrap_or(1);
            assert!(
                code == 0 || code == 1,
                "Expected exit code 0 or 1 on truncated file, got {}",
                code
            );
        }
    }
}

#[test]
fn test_strip() {
    let fixtures = get_fixtures();
    if fixtures.is_empty() {
        return;
    }

    for path in fixtures {
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if ext != "jpg" && ext != "jpeg" && ext != "png" {
            continue;
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join(path.file_name().unwrap());
        std::fs::copy(&path, &test_file).unwrap();

        // run strip inplace
        Command::cargo_bin("ime")
            .unwrap()
            .arg(&test_file)
            .arg("-s")
            .assert()
            .success();

        // read back
        Command::cargo_bin("ime")
            .unwrap()
            .arg(&test_file)
            .assert()
            .success()
            .stdout(predicate::str::contains("{}"));
    }
}
