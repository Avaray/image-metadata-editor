use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Create a minimal valid 1×1 JPEG file (no metadata).
fn minimal_jpeg(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    // SOI + minimal JFIF APP0 + SOF0 + EOI (enough for little_exif to open)
    // We copy from the existing fixture instead, which is always valid.
    fs::copy("tests/fixtures/00967-3353796624.jpg", &path).unwrap();
    path
}

/// Create a minimal valid 1×1 PNG file by copying from an existing fixture.
fn minimal_png(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy("tests/fixtures/00021-3108185260.png", &path).unwrap();
    path
}

/// Create a minimal valid WebP file (1×1, lossy VP8).
fn minimal_webp(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    // Minimal 1×1 lossy WebP with a known-good VP8 bitstream.
    // VP8 frame layout:
    //   [0..3]  frame tag: 0x30 0x01 0x00 (key frame, no version, no show_frame, part_size=3)
    //   [3..6]  start code: 0x9D 0x01 0x2A
    //   [6..8]  width/scale  LE: 0x01 0x00 → width  = 1
    //   [8..10] height/scale LE: 0x01 0x00 → height = 1
    //   [10..]  compressed data (DCT coefficients for a 1×1 block)
    let vp8_bitstream: &[u8] = &[
        0x30, 0x01, 0x00, // frame tag
        0x9d, 0x01, 0x2a, // start code
        0x01, 0x00, // width  = 1
        0x01, 0x00, // height = 1
        // minimal bitstream payload (empty macroblock + end-of-partition)
        0x00, 0x34, 0x25, 0xa4, 0x00, 0x03, 0x70, 0x00, 0xfe, 0xfb, 0x94, 0x00, 0x00,
    ];
    let vp8_data_len = vp8_bitstream.len() as u32; // unpadded size written in chunk header
    let vp8_padded = vp8_data_len + (vp8_data_len % 2); // even-aligned for RIFF

    // RIFF total size = "WEBP"(4) + "VP8 "(4) + chunk_size_field(4) + vp8_padded
    let riff_size: u32 = 4 + 4 + 4 + vp8_padded;

    let mut data: Vec<u8> = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&riff_size.to_le_bytes());
    data.extend_from_slice(b"WEBP");
    data.extend_from_slice(b"VP8 ");
    data.extend_from_slice(&vp8_data_len.to_le_bytes()); // chunk payload size (unpadded)
    data.extend_from_slice(vp8_bitstream);
    if vp8_data_len % 2 != 0 {
        data.push(0); // padding byte
    }
    fs::write(&path, &data).unwrap();
    path
}

/// Run `ime <path>` and return parsed JSON output.
fn read_metadata(path: &std::path::Path) -> serde_json::Value {
    let output = Command::cargo_bin("ime").unwrap().arg(path).output().unwrap();

    assert!(output.status.success(), "ime read failed for {:?}: {}", path, String::from_utf8_lossy(&output.stderr));

    serde_json::from_slice(&output.stdout).expect("ime output is not valid JSON")
}

/// Run `ime <path> --set KEY=VALUE` in-place.
fn inject_tag(path: &std::path::Path, key: &str, value: &str) {
    let result = Command::cargo_bin("ime").unwrap().arg(path).arg("--set").arg(format!("{}={}", key, value)).assert();

    result.success();
}

/// Flatten all directories in the JSON output into a single map: tag → value.
fn flatten_json(json: &serde_json::Value) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(obj) = json.as_object() {
        for (_dir, tags) in obj {
            if let Some(tags_obj) = tags.as_object() {
                for (tag, val) in tags_obj {
                    if let Some(s) = val.as_str() {
                        map.insert(tag.clone(), s.to_string());
                    }
                }
            }
        }
    }
    map
}

// ── JPEG tests ────────────────────────────────────────────────────────────────

/// Write a known EXIF tag (Artist) to a JPEG and read it back.
#[test]
fn test_inject_jpeg_exif_tag_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_jpeg(dir.path(), "test.jpg");

    inject_tag(&file, "Artist", "Test Artist");

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("Artist").map(String::as_str), Some("Test Artist"), "JPEG: Artist EXIF tag not found after inject.\nFull metadata: {:#?}", flat);
}

/// Write multiple EXIF tags to a JPEG in one command.
#[test]
fn test_inject_jpeg_multiple_tags_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_jpeg(dir.path(), "test.jpg");

    Command::cargo_bin("ime").unwrap().arg(&file).arg("--set").arg("Artist=Photographer Name").arg("--set").arg("Copyright=2024 Some Studio").arg("--set").arg("Software=TestTool v1.0").assert().success();

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("Artist").map(String::as_str), Some("Photographer Name"), "JPEG: Artist missing");
    assert_eq!(flat.get("Copyright").map(String::as_str), Some("2024 Some Studio"), "JPEG: Copyright missing");
    assert_eq!(flat.get("Software").map(String::as_str), Some("TestTool v1.0"), "JPEG: Software missing");
}

/// Unknown tags (non-EXIF) on JPEG are stored in UserComment as JSON.
/// After inject, reading back should yield them under UserComment or in a flat key.
#[test]
fn test_inject_jpeg_unknown_tag_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_jpeg(dir.path(), "test.jpg");

    inject_tag(&file, "prompt", "a beautiful sunset");

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    // Unknown tags are packed into UserComment as JSON {"prompt":"a beautiful sunset"}
    // They should appear in UserComment or unwrapped as a top-level flat key.
    let found = flat.get("UserComment").map(|v| v.contains("a beautiful sunset")).unwrap_or(false) || flat.get("prompt").map(String::as_str) == Some("a beautiful sunset");

    assert!(found, "JPEG: unknown tag 'prompt' not found after inject.\nFull metadata: {:#?}", flat);
}

/// Inject then strip should yield empty metadata.
#[test]
fn test_inject_jpeg_then_strip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_jpeg(dir.path(), "test.jpg");

    inject_tag(&file, "Artist", "Someone");

    // Verify it was written
    let json = read_metadata(&file);
    let flat = flatten_json(&json);
    assert!(flat.contains_key("Artist"), "JPEG: Artist not written before strip");

    // Strip
    Command::cargo_bin("ime").unwrap().arg(&file).arg("-s").assert().success();

    // Should now be empty
    let output = Command::cargo_bin("ime").unwrap().arg(&file).output().unwrap();
    assert!(output.status.success());
    let stripped: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stripped, serde_json::json!({}), "JPEG: metadata not empty after strip");
}

/// Verify that inject writes to a separate output file, leaving the source untouched.
#[test]
fn test_inject_jpeg_output_to_separate_file() {
    let dir = tempfile::tempdir().unwrap();
    let src = minimal_jpeg(dir.path(), "src.jpg");
    let dst = dir.path().join("dst.jpg");

    Command::cargo_bin("ime").unwrap().arg(&src).arg("--set").arg("Artist=Separate Output").arg("-o").arg(&dst).assert().success();

    assert!(dst.exists(), "JPEG: output file not created");

    // Destination should have Artist
    let dst_json = read_metadata(&dst);
    let dst_flat = flatten_json(&dst_json);
    assert_eq!(dst_flat.get("Artist").map(String::as_str), Some("Separate Output"), "JPEG: Artist not in output file");

    // Source should NOT have Artist (read its original fixture, which may have tags but not Artist="Separate Output")
    let src_json = read_metadata(&src);
    let src_flat = flatten_json(&src_json);
    assert_ne!(src_flat.get("Artist").map(String::as_str), Some("Separate Output"), "JPEG: source was modified unexpectedly");
}

// ── PNG tests ─────────────────────────────────────────────────────────────────

/// Write a known PngText tag to a PNG and read it back.
#[test]
fn test_inject_png_text_tag_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_png(dir.path(), "test.png");

    inject_tag(&file, "prompt", "a beautiful sunset");

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("prompt").map(String::as_str), Some("a beautiful sunset"), "PNG: PngText 'prompt' not found after inject.\nFull metadata: {:#?}", flat);
}

/// Write a PngText tag and an EXIF tag simultaneously to a PNG.
#[test]
fn test_inject_png_mixed_tags_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_png(dir.path(), "test.png");

    Command::cargo_bin("ime").unwrap().arg(&file).arg("--set").arg("prompt=portrait photo").arg("--set").arg("Artist=PNG Artist").assert().success();

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    println!("PNG MIXED TAGS FLAT: {:#?}", flat);
    assert_eq!(flat.get("prompt").map(String::as_str), Some("portrait photo"), "PNG: PngText 'prompt' missing");
    assert_eq!(flat.get("Artist").map(String::as_str), Some("PNG Artist"), "PNG: EXIF Artist missing");
}

/// Overwrite an existing PngText tag.
#[test]
fn test_inject_png_overwrite_existing_tag() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_png(dir.path(), "test.png");

    inject_tag(&file, "comment", "first value");
    inject_tag(&file, "comment", "second value");

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("comment").map(String::as_str), Some("second value"), "PNG: tag not overwritten correctly.\nFull metadata: {:#?}", flat);
}

/// Inject then strip should yield empty metadata for PNG.
#[test]
fn test_inject_png_then_strip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_png(dir.path(), "test.png");

    inject_tag(&file, "prompt", "test prompt");

    let output = Command::cargo_bin("ime").unwrap().arg(&file).arg("-s").assert().success();
    let _ = output;

    let stripped: serde_json::Value = serde_json::from_slice(&Command::cargo_bin("ime").unwrap().arg(&file).output().unwrap().stdout).unwrap();
    assert_eq!(stripped, serde_json::json!({}), "PNG: metadata not empty after strip");
}

/// Delete a specific PngText key.
#[test]
fn test_delete_png_text_key() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_png(dir.path(), "test.png");

    Command::cargo_bin("ime").unwrap().arg(&file).arg("--set").arg("alpha=keep").arg("--set").arg("beta=remove").assert().success();

    Command::cargo_bin("ime").unwrap().arg(&file).arg("--delete").arg("PngText.beta").assert().success();

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("alpha").map(String::as_str), Some("keep"), "PNG: 'alpha' was removed unexpectedly");
    assert!(flat.get("beta").is_none(), "PNG: 'beta' was not deleted.\nFull metadata: {:#?}", flat);
}

// ── WebP tests ────────────────────────────────────────────────────────────────

/// Write a known EXIF tag to a WebP and read it back.
#[test]
fn test_inject_webp_exif_tag_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_webp(dir.path(), "test.webp");

    inject_tag(&file, "Artist", "WebP Artist");

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("Artist").map(String::as_str), Some("WebP Artist"), "WebP: Artist EXIF tag not found after inject.\nFull metadata: {:#?}", flat);
}

/// Unknown tags on WebP are stored in UserComment as JSON — verify round-trip.
#[test]
fn test_inject_webp_unknown_tag_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_webp(dir.path(), "test.webp");

    inject_tag(&file, "custom_key", "custom_value");

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    let found = flat.get("UserComment").map(|v| v.contains("custom_value")).unwrap_or(false) || flat.get("custom_key").map(String::as_str) == Some("custom_value");

    assert!(found, "WebP: unknown tag 'custom_key' not found after inject.\nFull metadata: {:#?}", flat);
}

/// Write multiple EXIF tags to a WebP.
#[test]
fn test_inject_webp_multiple_exif_tags_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = minimal_webp(dir.path(), "test.webp");

    Command::cargo_bin("ime").unwrap().arg(&file).arg("--set").arg("Artist=WebP Photographer").arg("--set").arg("Copyright=2024 WebP Studio").assert().success();

    let json = read_metadata(&file);
    let flat = flatten_json(&json);

    assert_eq!(flat.get("Artist").map(String::as_str), Some("WebP Photographer"), "WebP: Artist missing");
    assert_eq!(flat.get("Copyright").map(String::as_str), Some("2024 WebP Studio"), "WebP: Copyright missing");
}

// ── Cross-format correctness ──────────────────────────────────────────────────

/// The same tag name/value injected into all three formats should always be readable back.
#[test]
fn test_inject_artist_all_formats_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let cases: &[(&str, fn(&std::path::Path, &str) -> PathBuf)] = &[("artist_test.jpg", minimal_jpeg), ("artist_test.png", minimal_png), ("artist_test.webp", minimal_webp)];

    for (filename, make_file) in cases {
        let file = make_file(dir.path(), filename);
        inject_tag(&file, "Artist", "Cross-Format Test");

        let json = read_metadata(&file);
        let flat = flatten_json(&json);

        assert_eq!(flat.get("Artist").map(String::as_str), Some("Cross-Format Test"), "{}: Artist not readable after inject.\nFull metadata: {:#?}", filename, flat);
    }
}
