// Each test target compiles this module separately and uses a subset.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

pub fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

/// Copy a fixture into a fresh temp dir and return the working path.
pub fn work_copy(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::copy(fixture(name), &path).unwrap();
    (dir, path)
}

pub fn run_ime() -> Command {
    Command::cargo_bin("ime").unwrap()
}

/// Read a file's metadata through the binary, asserting success and valid JSON.
pub fn read_json(path: &Path) -> Value {
    let assert = run_ime().arg(path).assert().success();
    let output = assert.get_output();
    assert!(output.stderr.is_empty(), "stderr must be empty on success: {:?}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON")
}

/// Independent image-data extractors (duplicated parsing on purpose: they are
/// the oracle for post-write verification and wipe tests, not users of it).
pub fn png_idat(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(&bytes[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut out = Vec::new();
    let mut pos = 8;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        if &bytes[pos + 4..pos + 8] == b"IDAT" {
            out.extend_from_slice(&bytes[pos + 8..pos + 8 + len]);
        }
        if &bytes[pos + 4..pos + 8] == b"IEND" {
            break;
        }
        pos += 12 + len;
    }
    out
}

pub fn png_chunk_types(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 8;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let kind = String::from_utf8_lossy(&bytes[pos + 4..pos + 8]).into_owned();
        out.push(kind.clone());
        if kind == "IEND" {
            break;
        }
        pos += 12 + len;
    }
    out
}

pub fn png_text_payload(bytes: &[u8], keyword: &str) -> Option<Vec<u8>> {
    let mut pos = 8;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        if &bytes[pos + 4..pos + 8] == b"tEXt" {
            let data = &bytes[pos + 8..pos + 8 + len];
            if let Some(nul) = data.iter().position(|&b| b == 0)
                && &data[..nul] == keyword.as_bytes()
            {
                return Some(data[nul + 1..].to_vec());
            }
        }
        if &bytes[pos + 4..pos + 8] == b"IEND" {
            break;
        }
        pos += 12 + len;
    }
    None
}

pub fn jpeg_markers(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    let mut out = Vec::new();
    let mut pos = 2;
    while pos + 1 < bytes.len() {
        if bytes[pos] != 0xFF {
            break;
        }
        while bytes[pos] == 0xFF {
            pos += 1;
        }
        let code = bytes[pos];
        out.push(code);
        if code == 0xD9 || code == 0xDA {
            break;
        }
        if code == 0x01 || (0xD0..0xD8).contains(&code) {
            pos += 1;
            continue;
        }
        let len = u16::from_be_bytes([bytes[pos + 1], bytes[pos + 2]]) as usize;
        pos += 1 + len;
    }
    out
}

pub fn jpeg_scan(bytes: &[u8]) -> Vec<u8> {
    // Slice from the first SOS marker to EOI (inclusive of SOS, exclusive of EOI).
    let mut pos = 2;
    let sos = loop {
        assert_eq!(bytes[pos], 0xFF, "expected marker");
        let mut p = pos;
        while bytes[p] == 0xFF {
            p += 1;
        }
        let code = bytes[p];
        assert_ne!(code, 0xD9, "EOI before SOS");
        if code == 0xDA {
            break pos;
        }
        if code == 0x01 || (0xD0..0xD8).contains(&code) {
            pos = p + 1;
        } else {
            let len = u16::from_be_bytes([bytes[p + 1], bytes[p + 2]]) as usize;
            pos = p + 1 + len;
        }
    };
    let sos_len = u16::from_be_bytes([bytes[sos + 2], bytes[sos + 3]]) as usize;
    let mut i = sos + 2 + sos_len;
    while i + 1 < bytes.len() {
        if bytes[i] == 0xFF {
            if bytes[i + 1] == 0x00 {
                i += 2;
            } else if bytes[i + 1] == 0xD9 {
                return bytes[sos..i].to_vec();
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    panic!("missing EOI");
}

pub fn webp_chunks(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    let mut out = Vec::new();
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let len = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        out.push((String::from_utf8_lossy(&bytes[pos..pos + 4]).into_owned(), bytes[pos + 8..pos + 8 + len].to_vec()));
        pos += 8 + len + (len % 2);
    }
    out
}

pub fn webp_image_data(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (fourcc, data) in webp_chunks(bytes) {
        if ["VP8 ", "VP8L", "ALPH", "ANMF"].contains(&fourcc.as_str()) {
            out.extend_from_slice(&data);
        }
    }
    out
}
