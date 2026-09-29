mod common;

use predicates::prelude::*;
use serde_json::{Value, json};

use common::{fixture, read_json, run_ime, work_copy};

fn photo_names() -> [&'static str; 3] {
    ["photo.png", "photo.jpg", "photo.webp"]
}

/// The custom-section key valid for a fixture: `PngText` for PNG files,
/// `UserComment` for JPEG/WebP.
fn ck(name: &str) -> &'static str {
    if name.ends_with(".png") { "PngText" } else { "UserComment" }
}

/// Wrap an inner custom object into a full `--set` payload for the fixture's format.
fn custom_payload(name: &str, inner: &str) -> String {
    ["{\"custom\": {\"", ck(name), "\": ", inner, "}}"].concat()
}

/// Full `--set` payload with an exif object and a format-appropriate custom object.
fn set_payload(exif_inner: &str, name: &str, custom_inner: &str) -> String {
    ["{\"exif\": ", exif_inner, ", \"custom\": {\"", ck(name), "\": ", custom_inner, "}}"].concat()
}

#[test]
fn set_merges_leaving_others_untouched() {
    for name in photo_names() {
        let (_dir, path) = work_copy(name);
        let before = read_json(&path);
        let payload = set_payload(r#"{"Make": "Merged"}"#, name, r#"{"added": {"deep": [1, 2]}}"#);
        run_ime().arg(&path).arg("--set").arg(payload).assert().success().code(0).stdout(predicate::str::is_empty());
        let after = read_json(&path);

        let mut expected_exif = before["exif"].as_object().unwrap().clone();
        expected_exif.insert("Make".to_string(), Value::String("Merged".to_string()));
        let mut expected_inner = before["custom"][ck(name)].as_object().unwrap().clone();
        expected_inner.insert("added".to_string(), json!({"deep": [1, 2]}));
        assert_eq!(after["exif"], Value::Object(expected_exif), "{name}: only Make may change");
        assert_eq!(after["custom"][ck(name)], Value::Object(expected_inner), "{name}: only `added` may change");
    }
}

#[test]
fn set_null_and_empty_values_delete() {
    for name in photo_names() {
        let (_dir, path) = work_copy(name);
        // All four empty spellings across the two custom shapes: PNG deletes
        // whole chunks, JPEG/WebP delete keys inside the comment document.
        let inner = if name.ends_with(".png") { r#"{"comment": null, "workflow": {}}"# } else { r#"{"rating": "", "nested": []}"# };
        let payload = set_payload(r#"{"Model": null, "ISO": "", "Orientation": {}, "Software": []}"#, name, inner);
        run_ime().arg(&path).arg("--set").arg(payload).assert().success();
        let after = read_json(&path);
        for key in ["Model", "ISO", "Orientation", "Software"] {
            assert!(!after["exif"].as_object().unwrap().contains_key(key), "{name}: exif.{key} must be deleted");
        }
        assert_eq!(after["exif"]["Make"], Value::String("TestMake".to_string()), "{name}: untouched keys stay");
        if name.ends_with(".png") {
            assert!(!after.as_object().unwrap().contains_key("custom"), "{name}: all chunks gone, section must be omitted");
        } else {
            let comment = after["custom"]["UserComment"].as_object().unwrap();
            assert!(!comment.contains_key("rating") && !comment.contains_key("nested"), "{name}: deleted keys must be gone");
            assert_eq!(comment["project"], Value::String("stage1".to_string()), "{name}: untouched keys stay");
        }
    }
}

#[test]
fn set_section_null_clears_only_that_section() {
    let (_dir, png) = work_copy("photo.png");
    run_ime().arg(&png).arg("--set").arg(r#"{"exif": null}"#).assert().success();
    let after = read_json(&png);
    assert!(!after.as_object().unwrap().contains_key("exif"), "exif section must be gone");
    assert_eq!(after["custom"]["PngText"]["comment"], Value::String("a stage1 photo".to_string()), "custom section must be untouched");

    let (_dir, webp) = work_copy("photo.webp");
    run_ime().arg(&webp).arg("--set").arg(r#"{"custom": null}"#).assert().success();
    let after = read_json(&webp);
    assert!(!after.as_object().unwrap().contains_key("custom"), "custom section must be gone");
    assert_eq!(after["exif"]["Make"], Value::String("TestMake".to_string()), "exif section must be untouched");
}

#[test]
fn set_multiple_occurrences_apply_in_order() {
    let (_dir, path) = work_copy("photo.jpg");
    run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "First"}, "custom": {"UserComment": {"a": 1, "b": 1}}}"#).arg("--set").arg(r#"{"exif": {"Make": "Second"}, "custom": {"UserComment": {"b": null}}}"#).assert().success();
    let after = read_json(&path);
    assert_eq!(after["exif"]["Make"], Value::String("Second".to_string()));
    assert_eq!(after["custom"]["UserComment"]["a"], json!(1));
    assert!(!after["custom"]["UserComment"].as_object().unwrap().contains_key("b"));
}

#[test]
fn set_accepts_json5_syntax() {
    let (_dir, path) = work_copy("photo.png");
    run_ime().arg(&path).arg("--set").arg("{exif: {Make: 'Single', ISO: 200,}, custom: {PngText: {trailing: true,},},}").assert().success();
    let after = read_json(&path);
    assert_eq!(after["exif"]["Make"], Value::String("Single".to_string()));
    assert_eq!(after["exif"]["ISO"], json!(200));
    // Chunk text is text: a bare JSON scalar round-trips as its string form.
    assert_eq!(after["custom"]["PngText"]["trailing"], Value::String("true".to_string()));
}

#[test]
fn set_invalid_json_leaves_file_untouched() {
    let (dir, path) = work_copy("photo.jpg");
    let before = std::fs::read(&path).unwrap();
    run_ime().arg(&path).arg("--set").arg("{not valid").assert().failure().code(1).stdout(predicate::str::is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before, "file must be untouched");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp files may be left behind");
}

#[test]
fn set_rejects_unknown_exif_tags_and_usercomment() {
    for payload in [r#"{"exif": {"NoSuchTag": 1}}"#, r#"{"exif": {"UserComment": "hi"}}"#, r#"{"exif": {"ExifOffset": 5}}"#] {
        let (dir, path) = work_copy("photo.png");
        let before = std::fs::read(&path).unwrap();
        run_ime().arg(&path).arg("--set").arg(payload).assert().failure().code(1).stdout(predicate::str::is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before, "file must be untouched for {payload}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp files may be left behind");
    }
}

#[test]
fn set_rejects_wrong_custom_keys_and_values() {
    for (name, payload) in [("photo.png", r#"{"custom": {"UserComment": "x"}}"#), ("photo.jpg", r#"{"custom": {"PngText": {"a": 1}}}"#), ("photo.webp", r#"{"custom": {"PngText": {"a": 1}}}"#), ("photo.png", r#"{"custom": {"PngText": "not-an-object"}}"#), ("photo.png", r#"{"custom": {"PngText": {"Raw profile type exif": "x"}}}"#), ("photo.jpg", r#"{"custom": {"UserComment": "zażółć"}}"#)] {
        let (dir, path) = work_copy(name);
        let before = std::fs::read(&path).unwrap();
        run_ime().arg(&path).arg("--set").arg(payload).assert().failure().code(1).stdout(predicate::str::is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before, "file must be untouched for {payload}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp files may be left behind");
    }
}

#[test]
fn set_merges_deep_inside_a_png_chunk_document() {
    let (_dir, path) = work_copy("photo.png");
    run_ime().arg(&path).arg("--set").arg(r#"{"custom": {"PngText": {"workflow": {"rating": 9}}}}"#).assert().success();
    let after = read_json(&path)["custom"]["PngText"].clone();
    assert_eq!(after["workflow"]["rating"], json!(9));
    assert_eq!(after["workflow"]["project"], Value::String("stage1".to_string()), "sibling keys stay");
    assert!(after["workflow"]["nested"].is_object(), "sibling keys stay");
    assert_eq!(after["comment"], Value::String("a stage1 photo".to_string()), "other chunks stay");
}

#[test]
fn set_usercomment_transitions_between_string_and_object() {
    let (_dir, note) = work_copy("note.jpg");
    run_ime().arg(&note).arg("--set").arg(r#"{"custom": {"UserComment": {"a": 1}}}"#).assert().success();
    assert_eq!(read_json(&note)["custom"]["UserComment"], json!({"a": 1}));

    let (_dir, photo) = work_copy("photo.jpg");
    run_ime().arg(&photo).arg("--set").arg(r#"{"custom": {"UserComment": "just text"}}"#).assert().success();
    assert_eq!(read_json(&photo)["custom"]["UserComment"], Value::String("just text".to_string()));
}

#[test]
fn set_pngtext_writes_itxt_chunks() {
    let (_dir, path) = work_copy("bare.png");
    run_ime().arg(&path).arg("--set").arg(r#"{"custom": {"PngText": {"b": "second", "a": "first"}}}"#).assert().success();
    let bytes = std::fs::read(&path).unwrap();
    assert!(common::png_chunk_types(&bytes).contains(&"iTXt".to_string()), "custom writes must use iTXt");
    assert_eq!(common::png_itxt_text(&bytes, "a").as_deref(), Some("first"));
    assert_eq!(common::png_itxt_text(&bytes, "b").as_deref(), Some("second"));
    // Keywords land sorted for deterministic output regardless of input order.
    let kinds: Vec<String> = common::png_itxt_keywords(&bytes);
    assert_eq!(kinds, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn set_rejects_wrong_value_types() {
    for payload in [r#"{"exif": {"Orientation": "not-a-number"}}"#, r#"{"exif": {"Orientation": 70000}}"#, r#"{"exif": {"ExposureTime": "soon"}}"#, r#"{"exif": {"ExposureTime": "1/0"}}"#, r#"{"exif": {"GPSLatitude": ["1/1", "2/1"]}}"#, r#"{"exif": {"Make": 42}}"#, r#"{"exif": {"Make": "Zażółć"}}"#, r#"{"exif": {"GPSLatitudeRef": "North"}}"#, r#"{"custom": 5}"#, r#"{"bogus": {}}"#, r#"[1, 2]"#] {
        let (_dir, path) = work_copy("photo.jpg");
        let before = std::fs::read(&path).unwrap();
        run_ime().arg(&path).arg("--set").arg(payload).assert().failure().code(1).stdout(predicate::str::is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before, "file must be untouched for {payload}");
    }
}

#[test]
fn set_on_bare_files_creates_metadata() {
    for name in ["bare.png", "bare.jpg", "bare.webp", "simple_vp8.webp", "simple_vp8l.webp"] {
        let (_dir, path) = work_copy(name);
        let payload = if name.ends_with(".png") { r#"{"exif": {"Make": "Born", "ISO": 100}, "custom": {"PngText": {"k": "v"}}}"#.to_string() } else { r#"{"exif": {"Make": "Born", "ISO": 100}, "custom": {"UserComment": "v"}}"#.to_string() };
        run_ime().arg(&path).arg("--set").arg(payload).assert().success();
        let after = read_json(&path);
        assert_eq!(after["exif"]["Make"], Value::String("Born".to_string()), "{name}");
        assert_eq!(after["exif"]["ISO"], json!(100), "{name}");
        if name.ends_with(".png") {
            assert_eq!(after["custom"]["PngText"]["k"], Value::String("v".to_string()), "{name}");
        } else {
            assert_eq!(after["custom"]["UserComment"], Value::String("v".to_string()), "{name}");
        }
    }
}

#[test]
fn output_leaves_original_untouched() {
    let (_dir, path) = work_copy("photo.png");
    let before = std::fs::read(&path).unwrap();
    let out = path.with_extension("out.png");
    run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Copy"}}"#).arg("--output").arg(&out).assert().success().stdout(predicate::str::is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before, "original must be byte-identical");
    assert_eq!(read_json(&out)["exif"]["Make"], Value::String("Copy".to_string()));

    // Existing destination is overwritten silently.
    run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Again"}}"#).arg("--output").arg(&out).assert().success();
    assert_eq!(read_json(&out)["exif"]["Make"], Value::String("Again".to_string()));
}

#[test]
fn output_dash_writes_image_to_stdout() {
    let (_dir, path) = work_copy("photo.jpg");
    let before = std::fs::read(&path).unwrap();
    let file_out = path.with_extension("out.jpg");
    run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Streamed"}}"#).arg("--output").arg(&file_out).assert().success();
    let file_bytes = std::fs::read(&file_out).unwrap();

    let assert = run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Streamed"}}"#).arg("--output").arg("-").assert().success();
    assert_eq!(assert.get_output().stdout, file_bytes, "stdout image must equal file output");
    assert_eq!(std::fs::read(&path).unwrap(), before, "original must be untouched");

    // The streamed bytes are a valid image carrying the change.
    let streamed = path.with_extension("streamed.jpg");
    std::fs::write(&streamed, &assert.get_output().stdout).unwrap();
    assert_eq!(read_json(&streamed)["exif"]["Make"], Value::String("Streamed".to_string()));
}

#[test]
fn stdin_image_with_stdout_output() {
    let input = std::fs::read(fixture("photo.png")).unwrap();
    let mut cmd = run_ime();
    let assert = cmd.arg("-").arg("--set").arg(r#"{"custom": {"PngText": {"pipe": "flowing"}}}"#).arg("--output").arg("-").write_stdin(input).assert().success();
    let dir = tempfile::tempdir().unwrap();
    let roundtrip = dir.path().join("roundtrip.png");
    std::fs::write(&roundtrip, &assert.get_output().stdout).unwrap();
    assert_eq!(read_json(&roundtrip)["custom"]["PngText"]["pipe"], Value::String("flowing".to_string()));
}

#[test]
fn set_is_deterministic() {
    for name in photo_names() {
        let (_dir, first) = work_copy(name);
        let (_dir2, second) = work_copy(name);
        let payload = set_payload(r#"{"Make": "Det", "GPSLatitude": ["9/1", "8/1", "7/1"]}"#, name, r#"{"b": 1, "a": [3, 2, 1]}"#);
        run_ime().arg(&first).arg("--set").arg(&payload).assert().success();
        run_ime().arg(&second).arg("--set").arg(&payload).assert().success();
        assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap(), "{name}: identical inputs must give identical files");

        // Custom key order in the input must not affect the output bytes.
        let (_dir3, third) = work_copy(name);
        run_ime().arg(&third).arg("--set").arg(custom_payload(name, r#"{"a": [3, 2, 1], "b": 1}"#)).assert().success();
        let (_dir4, fourth) = work_copy(name);
        run_ime().arg(&fourth).arg("--set").arg(custom_payload(name, r#"{"b": 1, "a": [3, 2, 1]}"#)).assert().success();
        assert_eq!(std::fs::read(&third).unwrap(), std::fs::read(&fourth).unwrap(), "{name}: custom key order must not affect bytes");
    }
}

#[test]
fn set_preserves_image_data() {
    for name in photo_names() {
        let (_dir, path) = work_copy(name);
        let before = std::fs::read(&path).unwrap();
        let payload = set_payload(r#"{"Make": "Pixels", "ISO": 3200}"#, name, r#"{"x": {"y": [true]}}"#);
        run_ime().arg(&path).arg("--set").arg(&payload).assert().success();
        let after = std::fs::read(&path).unwrap();
        if name.ends_with(".png") {
            assert_eq!(common::png_idat(&before), common::png_idat(&after), "PNG IDAT must be identical");
        } else if name.ends_with(".jpg") {
            assert_eq!(common::jpeg_scan(&before), common::jpeg_scan(&after), "JPEG scan data must be identical");
        } else {
            assert_eq!(common::webp_image_data(&before), common::webp_image_data(&after), "WebP image chunks must be identical");
        }
    }
}

#[test]
fn png_exif_roundtrips_through_ztxt() {
    // `little_exif` stores PNG EXIF in a zTXt chunk that `nom-exif` cannot
    // see; reads must still find it after a write.
    let (_dir, path) = work_copy("bare.png");
    run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Ztxt"}}"#).assert().success();
    let bytes = std::fs::read(&path).unwrap();
    assert!(common::png_chunk_types(&bytes).contains(&"zTXt".to_string()), "EXIF must be stored as zTXt");
    assert_eq!(read_json(&path)["exif"]["Make"], Value::String("Ztxt".to_string()));
}

#[test]
fn png_custom_only_set_preserves_exif_bytes() {
    let (_dir, path) = work_copy("photo.png");
    let before = read_json(&path)["exif"].clone();
    run_ime().arg(&path).arg("--set").arg(r#"{"custom": {"PngText": {"only": 1}}}"#).assert().success();
    assert_eq!(read_json(&path)["exif"], before, "EXIF values must be untouched");
    // The eXIf chunk itself must survive byte-for-byte (custom lives in tEXt).
    let after_bytes = std::fs::read(&path).unwrap();
    let mut pos = 8;
    let mut found = false;
    while pos + 8 <= after_bytes.len() {
        let len = u32::from_be_bytes([after_bytes[pos], after_bytes[pos + 1], after_bytes[pos + 2], after_bytes[pos + 3]]) as usize;
        if &after_bytes[pos + 4..pos + 8] == b"eXIf" {
            found = true;
        }
        if &after_bytes[pos + 4..pos + 8] == b"IEND" {
            break;
        }
        pos += 12 + len;
    }
    assert!(found, "eXIf chunk must still be present (not rewritten to zTXt)");
}

#[test]
fn webp_simple_files_are_promoted() {
    for name in ["simple_vp8.webp", "simple_vp8l.webp"] {
        let (_dir, path) = work_copy(name);
        let before = std::fs::read(&path).unwrap();
        run_ime().arg(&path).arg("--set").arg(r#"{"exif": {"Make": "Promoted"}}"#).assert().success();
        let after = std::fs::read(&path).unwrap();
        assert_eq!(&after[12..16], b"VP8X", "{name}: must gain a VP8X chunk");
        assert_eq!(read_json(&path)["exif"]["Make"], Value::String("Promoted".to_string()));
        assert_eq!(common::webp_image_data(&before), common::webp_image_data(&after), "{name}: image data must survive promotion");
    }
}

#[test]
fn usage_errors_for_bad_combinations() {
    let (_dir, path) = work_copy("photo.png");
    run_ime().arg(&path).arg("--set").arg("{}").arg("--wipe").assert().failure().code(2);
    run_ime().arg(&path).arg("--output").arg("x.png").assert().failure().code(2);
    run_ime().arg("--set").arg("{}").assert().failure().code(2);
}
