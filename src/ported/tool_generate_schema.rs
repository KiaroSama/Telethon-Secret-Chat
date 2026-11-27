//! Legacy schema helpers and checked input for the native maintenance command.

/// Validate complete constructor rows before generating a native manifest.
///
/// The historical adapter below intentionally retains its permissive behavior
/// for differential replay. The executable must not silently discard malformed
/// rows or collapse distinct declarations with the same numeric constructor ID.
pub fn inspect_checked_schema(text: String) -> Result<serde_json::Value, String> {
    let mut ids = std::collections::BTreeSet::new();
    let mut normalized = String::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split_once("//").map_or(raw, |(line, _)| line).trim();
        if line.is_empty() || matches!(line, "---types---" | "---functions---") {
            continue;
        }
        let invalid = || format!("invalid constructor declaration on line {}", index + 1);
        let declaration = line.strip_suffix(';').ok_or_else(invalid)?;
        if declaration.matches('=').count() != 1 {
            return Err(invalid());
        }
        let (head, result) = declaration.split_once('=').ok_or_else(invalid)?;
        if result.trim().is_empty() {
            return Err(invalid());
        }
        let token = head.split_whitespace().next().ok_or_else(invalid)?;
        let (name, id) = token.split_once('#').ok_or_else(invalid)?;
        if !name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.'))
            || id.is_empty()
            || id.len() > 8
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        let numeric_id = u32::from_str_radix(id, 16).map_err(|_| invalid())?;
        if !ids.insert(numeric_id) {
            return Err(format!("duplicate constructor ID on line {}", index + 1));
        }
        // Normalize whitespace for the frozen adapter's single-space tokenizer.
        normalized.push_str(&line.split_whitespace().collect::<Vec<_>>().join(" "));
        normalized.push('\n');
    }
    inspect_schema(normalized)
}

/// Port of `inspect_schema` (`rust/adapters/tool_generate_schema_adapter.py:9`).
pub fn inspect_schema(text: String) -> Result<serde_json::Value, String> {
    let entries: Vec<_> = text
        .lines()
        .filter_map(|raw| {
            let line = raw.trim();
            let line = line.strip_suffix(';')?;
            if line.starts_with("//") {
                return None;
            }
            let (head, _) = line.rsplit_once('=')?;
            let head = head.trim();
            let token = head.split(' ').next()?;
            let (name, id) = token.split_once('#')?;
            (!id.is_empty()).then(|| (name, id.to_lowercase()))
        })
        .collect();
    let last: std::collections::HashMap<_, _> = entries
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (*name, i))
        .collect();
    let mut names = serde_json::Map::new();
    for (i, (name, id)) in entries.iter().enumerate() {
        let mut chars = name.chars();
        let first = chars
            .next()
            .ok_or("IndexError: string index out of range")?;
        let base = format!("{}{}", first.to_uppercase(), chars.as_str());
        let display = if id == "aa48327d" {
            "DecryptedMessageService8".to_owned()
        } else if last.get(name) == Some(&i) {
            base
        } else {
            format!("{base}_{id}")
        };
        names.insert(id.clone(), serde_json::Value::String(display));
    }
    Ok(serde_json::json!({"count":entries.len(),"names":names}))
}

/// Port of `expressions` (`rust/adapters/tool_generate_schema_adapter.py:14`).
pub fn expressions(type_name: String, value: String) -> Result<serde_json::Value, String> {
    fn generate(kind: &str, value: &str) -> (Vec<String>, String) {
        let (write, read) = match kind {
            "int" => (format!("_pack_int({value})"), "r.read_int()"),
            "long" => (format!("_pack_long({value})"), "r.read_long()"),
            "double" => (format!("_pack_double({value})"), "r.read_double()"),
            "int128" => (format!("bytes({value})"), "r.read_large_int(bits=128)"),
            "int256" => (format!("bytes({value})"), "r.read_large_int(bits=256)"),
            "bytes" => (format!("_serialize_bytes({value})"), "r.tgread_bytes()"),
            "string" => (
                format!("_serialize_bytes({value})"),
                "r.tgread_bytes().decode(\"utf-8\")",
            ),
            "Bool" => (
                format!("_BOOL_TRUE if {value} else _BOOL_FALSE"),
                "r.tgread_bool()",
            ),
            _ => {
                if let Some(inner) = kind
                    .strip_prefix("Vector<")
                    .or_else(|| kind.strip_prefix("vector<"))
                    .and_then(|s| s.strip_suffix('>'))
                    .filter(|s| !s.is_empty())
                {
                    let (write, read) = generate(inner, "_item");
                    let mut lines = vec![
                        "w.append(_VECTOR)".to_owned(),
                        format!("w.append(_pack_int(len({value})))"),
                        format!("for _item in {value}:"),
                    ];
                    lines.extend(write.into_iter().map(|line| format!("    {line}")));
                    return (lines, format!("_read_vector(r, lambda: {read})"));
                }
                let quote = if kind.contains('\'') && !kind.contains('"') {
                    '"'
                } else {
                    '\''
                };
                let text: String = kind
                    .chars()
                    .map(|ch| match ch {
                        '\\' => "\\\\".into(),
                        '\n' => "\\n".into(),
                        '\r' => "\\r".into(),
                        '\t' => "\\t".into(),
                        c if c == quote => format!("\\{c}"),
                        c if c != ' '
                            && (c.is_whitespace()
                                || c.is_control()
                                || matches!(c, '\u{200b}'..='\u{200f}' | '\u{feff}')) =>
                        {
                            let n = u32::from(c);
                            if n <= 0xff {
                                format!("\\x{n:02x}")
                            } else {
                                format!("\\u{n:04x}")
                            }
                        }
                        c => c.to_string(),
                    })
                    .collect();
                return (
                    vec![format!("w.append(bytes({value}))")],
                    format!("read_object(r, expected={quote}{text}{quote})"),
                );
            }
        };
        (vec![format!("w.append({write})")], read.to_owned())
    }
    let (write, read) = generate(&type_name, &value);
    Ok(serde_json::json!({"write":write,"read":read}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_baseline_contract() -> Result<(), String> {
        let v = inspect_schema("dup#01 = X;\ndup#02 = X;".into())?;
        assert_eq!(v["names"]["01"], "Dup_01");
        assert_eq!(v["names"]["02"], "Dup");
        assert_eq!(
            expressions("int".into(), "x".into())?["read"],
            "r.read_int()"
        );
        Ok(())
    }
}
