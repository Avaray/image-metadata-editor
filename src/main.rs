mod cli;
mod error;
mod extract;
mod inject;
mod json_path;
mod model;
mod output;
mod strip;

use cli::CliResult;
use error::AppError;
use model::Output;
use std::collections::BTreeMap;
use std::process;

fn run() -> Result<(), AppError> {
    match cli::parse_args().map_err(|e| AppError::Usage(e.to_string()))? {
        CliResult::Help => {
            println!(
                "Usage: ime <file> [-o <path>] [-s] [--set K=V] [--set-json <json>] [-k <path>]"
            );
            println!("Options:");
            println!(
                "  -o, --output <path>   write result (or modified file) to <path>; default: stdout / in-place"
            );
            println!("  -s, --strip           strip all metadata (JPEG & PNG only)");
            println!("      --set K=V         set a metadata tag; can be repeated");
            println!("                        flat:    --set Artist=\"Jane\"");
            println!("                        nested:  --set .workflow.nodes[0].type=KSampler");
            println!("      --set-json <json> merge a JSON object into metadata");
            println!(
                "                        each top-level key becomes a tag; existing tags are preserved"
            );
            println!("  -k, --key <path>      extract a single value using dot-notation");
            println!(
                "                        e.g. -k Exif.Model  or  -k PngText.workflow.nodes[0].type"
            );
            println!(
                "                        string values that contain JSON are traversed automatically"
            );
            println!("  -v, --version         print version");
            println!("  -h, --help            print this help");
            Ok(())
        }
        CliResult::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        CliResult::Args(args) => {
            let has_inject = !args.set.is_empty() || args.set_json.is_some();

            // ── Step 1: strip (optional) ──────────────────────────────────────────
            // When chaining strip+inject we strip to a temp file, then inject from it.
            // When strip only, we strip directly to the final destination.
            let strip_temp: Option<String> = if args.strip && has_inject {
                let t = format!("{}.strip_tmp", args.file);
                strip::strip_metadata(&args.file, Some(&t))
                    .map_err(|e| AppError::Runtime(e.to_string()))?;
                Some(t)
            } else if args.strip {
                strip::strip_metadata(&args.file, args.output.as_deref())
                    .map_err(|e| AppError::Runtime(e.to_string()))?;
                return Ok(());
            } else {
                None
            };

            // The effective source for inject/read is either the stripped temp or the original.
            let source = strip_temp.as_deref().unwrap_or(&args.file);

            // ── Step 2: inject (optional) ─────────────────────────────────────────
            if has_inject {
                let mut parser = nom_exif::MediaParser::new();

                let flat_tags =
                    build_inject_tags(source, &args.set, args.set_json.as_deref(), &mut parser)
                        .map_err(AppError::Runtime)?;

                let dest = if strip_temp.is_some() && args.output.is_none() {
                    // strip produced a temp; inject should write back to the original file
                    Some(args.file.as_str())
                } else {
                    args.output.as_deref()
                };

                inject::inject_metadata(source, dest, &flat_tags)
                    .map_err(|e| AppError::Runtime(e.to_string()))?;

                // Clean up strip temp (inject already wrote the final file)
                if let Some(ref t) = strip_temp {
                    std::fs::remove_file(t).ok();
                }

                return Ok(());
            }

            // ── Step 3: read / query ──────────────────────────────────────────────
            let mut parser = nom_exif::MediaParser::new();
            let metadata = extract::extract(source, &mut parser)
                .map_err(|e| AppError::Runtime(e.to_string()))?;

            if let Some(key_path) = &args.key {
                let root: serde_json::Value = serde_json::to_value(&metadata)
                    .map_err(|e| AppError::Runtime(e.to_string()))?;
                match json_path::extract(root, key_path) {
                    Some(serde_json::Value::String(s)) => println!("{}", s),
                    Some(other) => {
                        let pretty = serde_json::to_string_pretty(&other)
                            .map_err(|e| AppError::Runtime(e.to_string()))?;
                        println!("{}", pretty);
                    }
                    None => {
                        return Err(AppError::Runtime(format!(
                            "Key '{}' not found in metadata",
                            key_path
                        )));
                    }
                }
                return Ok(());
            }

            output::write_output(&metadata, args.output.as_deref())
                .map_err(|e| AppError::Runtime(e.to_string()))?;
            Ok(())
        }
    }
}

/// Resolve all inject inputs (flat --set, nested --set .path=v, --set-json) into a single
/// flat BTreeMap<tag_name, string_value> ready to pass to inject::inject_metadata.
///
/// For nested paths the root tag's current value is read from `source_file`, parsed as JSON,
/// modified with json_path::set_at_path, and re-serialized back to a string.
fn build_inject_tags(
    source_file: &str,
    raw_set: &BTreeMap<String, String>,
    set_json: Option<&str>,
    parser: &mut nom_exif::MediaParser,
) -> Result<BTreeMap<String, String>, String> {
    let mut flat: BTreeMap<String, String> = BTreeMap::new();
    // root_tag → Vec<(sub_path, raw_value)>
    let mut nested: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    for (raw_key, value) in raw_set {
        // Strip optional leading dot then decide: flat or nested
        let key = raw_key.trim_start_matches('.');
        if let Some((root, sub)) = split_root_and_sub(key) {
            nested.entry(root).or_default().push((sub, value.clone()));
        } else {
            flat.insert(key.to_string(), value.clone());
        }
    }

    // ── Resolve nested paths ──────────────────────────────────────────────────
    if !nested.is_empty() {
        // Read current metadata once so we can find existing JSON string values
        let current: Output = extract::extract(source_file, parser)
            .map_err(|e| format!("Failed to read metadata for nested --set: {}", e))?;

        for (root_tag, modifications) in nested {
            // Look up the current string value of root_tag across all metadata directories
            let current_str = current
                .values()
                .find_map(|dir| dir.get(&root_tag))
                .map(String::as_str)
                .unwrap_or("{}");

            // Parse as JSON (fall back to empty object so we can build from scratch)
            let mut json: serde_json::Value = serde_json::from_str(current_str)
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            for (sub_path, raw_value) in modifications {
                // Try to parse the value as JSON so numbers/bools/objects are preserved;
                // fall back to a plain string.
                let new_val = serde_json::from_str(&raw_value)
                    .unwrap_or(serde_json::Value::String(raw_value));
                json = json_path::set_at_path(json, &sub_path, new_val);
            }

            flat.insert(root_tag, serde_json::to_string(&json).unwrap_or_default());
        }
    }

    // ── Resolve --set-json ────────────────────────────────────────────────────
    if let Some(json_str) = set_json {
        let obj: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| format!("--set-json: invalid JSON — {}", e))?;
        let map = obj.as_object().ok_or_else(|| {
            "--set-json: value must be a JSON object, not an array or scalar".to_string()
        })?;
        for (k, v) in map {
            let str_val = match v {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            // --set-json keys do NOT override explicit --set keys
            flat.entry(k.clone()).or_insert(str_val);
        }
    }

    Ok(flat)
}

/// Split a path like "workflow.nodes[0].type" into ("workflow", "nodes[0].type").
/// Returns None if there is no sub-path (plain flat key).
fn split_root_and_sub(path: &str) -> Option<(String, String)> {
    // Find first structural separator: '.' or '['
    let sep = path.find(['.', '['])?;
    let root = path[..sep].to_string();
    let sub = if path.as_bytes()[sep] == b'.' {
        path[sep + 1..].to_string()
    } else {
        // keep '[' as the start of the sub-path
        path[sep..].to_string()
    };
    if root.is_empty() {
        None
    } else {
        Some((root, sub))
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e);
        process::exit(e.exit_code());
    }
}
