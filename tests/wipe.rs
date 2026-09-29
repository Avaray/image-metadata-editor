mod common;

use predicates::prelude::*;
use serde_json::Value;

use common::{read_json, run_ime, work_copy};

#[test]
fn wipe_empties_metadata() {
    for name in ["photo.png", "photo.jpg", "photo.webp", "ancillary.png", "extra.jpg", "meta.webp", "anim.webp"] {
        let (_dir, path) = work_copy(name);
        run_ime().arg(&path).arg("--wipe").assert().success().code(0).stdout(predicate::str::is_empty());
        assert_eq!(read_json(&path), Value::Object(Default::default()), "{name}: wiped file must read back empty");
    }
}

#[test]
fn wipe_preserves_image_data() {
    for (name, extract) in [("photo.png", common::png_idat as fn(&[u8]) -> Vec<u8>), ("ancillary.png", common::png_idat), ("photo.jpg", common::jpeg_scan), ("extra.jpg", common::jpeg_scan), ("photo.webp", common::webp_image_data), ("meta.webp", common::webp_image_data), ("anim.webp", common::webp_image_data)] {
        let (_dir, path) = work_copy(name);
        let before = extract(&std::fs::read(&path).unwrap());
        assert!(!before.is_empty(), "{name}: fixture must have image data");
        run_ime().arg(&path).arg("--wipe").assert().success();
        assert_eq!(extract(&std::fs::read(&path).unwrap()), before, "{name}: image data must be bit-identical");
    }
}

#[test]
fn wipe_png_keeps_only_critical_chunks() {
    let (_dir, path) = work_copy("ancillary.png");
    run_ime().arg(&path).arg("--wipe").assert().success();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(common::png_chunk_types(&bytes), vec!["IHDR".to_string(), "IDAT".to_string(), "IEND".to_string()]);
}

#[test]
fn wipe_jpeg_removes_app_and_com_segments() {
    let (_dir, path) = work_copy("extra.jpg");
    assert!(common::jpeg_markers(&std::fs::read(&path).unwrap()).contains(&0xE0), "fixture must have APP0");
    run_ime().arg(&path).arg("--wipe").assert().success();
    for marker in common::jpeg_markers(&std::fs::read(&path).unwrap()) {
        assert!(!(0xE0..=0xEF).contains(&marker) && marker != 0xFE, "marker 0x{marker:02X} must be gone");
    }
}

#[test]
fn wipe_webp_removes_metadata_chunks_and_flags() {
    let (_dir, meta) = work_copy("meta.webp");
    run_ime().arg(&meta).arg("--wipe").assert().success();
    let bytes = std::fs::read(&meta).unwrap();
    let kinds: Vec<String> = common::webp_chunks(&bytes).into_iter().map(|(fourcc, _)| fourcc).collect();
    assert!(!kinds.contains(&"EXIF".to_string()) && !kinds.contains(&"ICCP".to_string()) && !kinds.contains(&"XMP ".to_string()), "metadata chunks must be gone: {kinds:?}");
    assert!(kinds.contains(&"VP8 ".to_string()) && kinds.contains(&"VP8X".to_string()), "image chunks must stay: {kinds:?}");
    assert_eq!(bytes[20] & 0x2C, 0, "VP8X metadata flags must be cleared");

    // Animation survives (frames are image data); only the EXIF flag clears.
    let (_dir, anim) = work_copy("anim.webp");
    run_ime().arg(&anim).arg("--wipe").assert().success();
    let bytes = std::fs::read(&anim).unwrap();
    let kinds: Vec<String> = common::webp_chunks(&bytes).into_iter().map(|(fourcc, _)| fourcc).collect();
    assert!(kinds.contains(&"ANMF".to_string()) && kinds.contains(&"ANIM".to_string()), "animation chunks must stay: {kinds:?}");
    assert_eq!(bytes[20], 0x02, "only the animation flag may remain");
}

#[test]
fn wipe_bare_files_is_a_noop() {
    for name in ["bare.png", "bare.jpg", "bare.webp"] {
        let (_dir, path) = work_copy(name);
        let before = std::fs::read(&path).unwrap();
        run_ime().arg(&path).arg("--wipe").assert().success();
        assert_eq!(std::fs::read(&path).unwrap(), before, "{name}: nothing to wipe, bytes must be identical");
    }
}

#[test]
fn wipe_with_output_leaves_original_untouched() {
    let (_dir, path) = work_copy("photo.jpg");
    let before = std::fs::read(&path).unwrap();
    let out = path.with_extension("wiped.jpg");
    run_ime().arg(&path).arg("--wipe").arg("--output").arg(&out).assert().success();
    assert_eq!(std::fs::read(&path).unwrap(), before, "original must be untouched");
    assert_eq!(read_json(&out), Value::Object(Default::default()));
}
