use serde_json::Value;

pub fn extract(mut current: Value, path: &str) -> Option<Value> {
    let mut current_segment = String::new();
    let mut segments = Vec::new();

    // Simple tokenizer for paths like "Exif.Model" or "PngText.workflow.nodes[0].type"
    for c in path.chars() {
        match c {
            '.' | '[' | ']' => {
                if !current_segment.is_empty() {
                    segments.push(current_segment.clone());
                    current_segment.clear();
                }
            }
            _ => current_segment.push(c),
        }
    }
    if !current_segment.is_empty() {
        segments.push(current_segment);
    }

    // Traverse the JSON Value
    for segment in segments {
        // If the current value is a string, attempt to parse it as JSON
        if current.is_string() {
            if let Ok(parsed) = serde_json::from_str::<Value>(current.as_str().unwrap()) {
                current = parsed;
            } else {
                return None; // Tried to step into a string that is not valid JSON
            }
        }

        match current {
            Value::Object(mut map) => {
                current = map.remove(&segment)?;
            }
            Value::Array(mut vec) => {
                let idx: usize = segment.parse().ok()?;
                if idx < vec.len() {
                    // swap_remove is O(1) and safe since we discard the rest of the array
                    current = vec.swap_remove(idx);
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }

    Some(current)
}
