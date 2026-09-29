use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

fn run_read(path: &Path) -> assert_cmd::assert::Assert {
    Command::cargo_bin("ime").unwrap().arg(path).assert()
}

fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    let assert = assert.success();
    let output = assert.get_output();
    assert!(output.stderr.is_empty(), "stderr must be empty on success: {:?}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON")
}

fn expected_custom() -> Value {
    serde_json::json!({"nested": {"a": [1, 2], "s": "x"}, "project": "stage1", "rating": 5})
}

fn expected_exif(gps_info_offset: u64) -> Value {
    serde_json::json!({
        "Artist": "404",
        "Copyright": "CC0 test fixture",
        "DateTimeOriginal": "2026:09:01 12:34:56",
        "ExifOffset": 252,
        "ExposureTime": "1/200",
        "FNumber": "28/10",
        "GPSInfo": gps_info_offset,
        "GPSLatitude": ["52/1", "13/1", "0/1"],
        "GPSLatitudeRef": "N",
        "GPSLongitude": ["21/1", "0/1", "0/1"],
        "GPSLongitudeRef": "E",
        "GPSVersionID": [2, 3, 0, 0],
        "ISO": 400,
        "ImageDescription": {"location": "studio", "tags": ["a", "b"]},
        "ImageWidth": 1,
        "Make": "TestMake",
        "Model": "TestModel X1",
        "Orientation": 6,
        "Software": "ime-fixture-gen",
        "XResolution": "300/1",
    })
}

#[test]
fn read_png_prints_exif_and_custom() {
    let value = stdout_json(run_read(&fixture("photo.png")));
    assert_eq!(value, serde_json::json!({"exif": expected_exif(342), "custom": expected_custom()}));
}

#[test]
fn read_jpeg_prints_exif_and_custom() {
    let value = stdout_json(run_read(&fixture("photo.jpg")));
    assert_eq!(value, serde_json::json!({"exif": expected_exif(422), "custom": expected_custom()}));
}

#[test]
fn read_webp_prints_exif_and_custom() {
    let value = stdout_json(run_read(&fixture("photo.webp")));
    assert_eq!(value, serde_json::json!({"exif": expected_exif(422), "custom": expected_custom()}));
}

#[test]
fn reads_file_with_misleading_extension() {
    let dir = tempfile::tempdir().unwrap();
    let renamed_png = dir.path().join("photo.dat");
    std::fs::copy(fixture("photo.png"), &renamed_png).unwrap();
    let value = stdout_json(run_read(&renamed_png));
    assert_eq!(value["exif"]["Make"], Value::String("TestMake".to_string()));

    let renamed_jpg = dir.path().join("photo.png");
    std::fs::copy(fixture("photo.jpg"), &renamed_jpg).unwrap();
    let value = stdout_json(run_read(&renamed_jpg));
    assert_eq!(value["exif"]["Make"], Value::String("TestMake".to_string()));
}

#[test]
fn rejects_non_image_file() {
    run_read(&fixture("not_an_image.txt")).failure().code(1).stdout(predicate::str::is_empty()).stderr(predicate::str::contains("unsupported image format"));
}

#[test]
fn rejects_corrupt_file() {
    run_read(&fixture("corrupt.jpg")).failure().code(1).stdout(predicate::str::is_empty()).stderr(predicate::str::contains("cannot read metadata"));
}

#[test]
fn bare_files_print_empty_object() {
    for name in ["bare.png", "bare.jpg", "bare.webp"] {
        let value = stdout_json(run_read(&fixture(name)));
        assert_eq!(value, Value::Object(Default::default()), "{name} must print an empty object");
    }
}

#[test]
fn traversal_embeds_objects_but_not_scalars() {
    let value = stdout_json(run_read(&fixture("photo.png")));
    assert!(value["exif"]["ImageDescription"].is_object(), "embedded JSON object must be traversed");
    assert_eq!(value["exif"]["Artist"], Value::String("404".to_string()), "numeric-looking string must stay a string");
}

#[test]
fn user_comment_never_appears_under_exif() {
    for name in ["photo.jpg", "photo.webp", "photo.png"] {
        let value = stdout_json(run_read(&fixture(name)));
        assert!(!value["exif"].as_object().unwrap().contains_key("UserComment"), "{name} must not expose UserComment under exif");
    }
}

#[test]
fn reads_image_from_stdin() {
    let bytes = std::fs::read(fixture("photo.jpg")).unwrap();
    let mut cmd = Command::cargo_bin("ime").unwrap();
    let assert = cmd.arg("-").write_stdin(bytes).assert();
    let value = stdout_json(assert);
    assert_eq!(value, serde_json::json!({"exif": expected_exif(422), "custom": expected_custom()}));
}

#[test]
fn exit_codes_for_missing_file_bad_flag_and_info_flags() {
    Command::cargo_bin("ime").unwrap().arg(fixture("does-not-exist.png")).assert().failure().code(1);
    Command::cargo_bin("ime").unwrap().arg(fixture("photo.png")).arg("--nope").assert().failure().code(2);
    Command::cargo_bin("ime").unwrap().arg("--help").assert().success().code(0);
    Command::cargo_bin("ime").unwrap().arg("--version").assert().success().code(0).stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
