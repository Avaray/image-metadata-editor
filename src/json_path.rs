use serde_json::Value;

/// Tokenize a jq-style path string into string segments.
/// "workflow.nodes[0].type" → ["workflow", "nodes", "0", "type"]
fn parse_segments(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    for c in path.chars() {
        match c {
            '.' | '[' | ']' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Read a value from a JSON tree using a dot/bracket path.
/// If a string value along the way looks like JSON, it is parsed and traversed.
pub fn extract(mut current: Value, path: &str) -> Option<Value> {
    for segment in parse_segments(path) {
        // Auto-parse embedded JSON strings
        if let Value::String(ref s) = current {
            if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                current = parsed;
            } else {
                return None;
            }
        }
        match current {
            Value::Object(mut map) => {
                current = map.remove(&segment)?;
            }
            Value::Array(mut vec) => {
                let idx: usize = segment.parse().ok()?;
                if idx < vec.len() {
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

/// Set a value inside a JSON tree at the given dot/bracket path.
/// Intermediate objects and arrays are created automatically.
/// Returns the modified root.
pub fn set_at_path(root: Value, path: &str, new_value: Value) -> Value {
    if path.is_empty() {
        return new_value;
    }
    let segments = parse_segments(path);
    set_recursive(root, &segments, new_value)
}

fn set_recursive(current: Value, segments: &[String], new_value: Value) -> Value {
    if segments.is_empty() {
        return new_value;
    }
    let segment = &segments[0];
    let rest = &segments[1..];

    if let Ok(idx) = segment.parse::<usize>() {
        // Array index
        let mut arr = if let Value::Array(a) = current { a } else { vec![] };
        // Grow array if needed
        while arr.len() <= idx {
            arr.push(Value::Null);
        }
        let child = std::mem::replace(&mut arr[idx], Value::Null);
        arr[idx] = set_recursive(child, rest, new_value);
        Value::Array(arr)
    } else {
        // Object key
        let mut obj = if let Value::Object(m) = current { m } else { serde_json::Map::new() };
        let child = obj.remove(segment).unwrap_or(Value::Null);
        let new_child = set_recursive(child, rest, new_value);
        obj.insert(segment.clone(), new_child);
        Value::Object(obj)
    }
}
