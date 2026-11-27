//! Native encoder and decoder for the frozen Telegram end-to-end TL schema.

use std::{collections::BTreeMap, sync::OnceLock};

use serde_json::{Map, Value, json};

type Outcome<T> = Result<T, String>;
const TL: &str = include_str!("../../schema/end-to-end.tl");
const VECTOR: u32 = 0x1cb5c415;
const BOOL_TRUE: u32 = 0x997275b5;
const BOOL_FALSE: u32 = 0xbc799737;

struct Field {
    name: &'static str,
    kind: &'static str,
    flag: Option<(&'static str, u32)>,
}

struct Ctor {
    id: u32,
    name: &'static str,
    family: &'static str,
    class: String,
    fields: Vec<Field>,
}

static CTORS: OnceLock<Outcome<Vec<Ctor>>> = OnceLock::new();

fn constructors() -> Outcome<&'static [Ctor]> {
    CTORS
        .get_or_init(|| {
            let mut rows = Vec::new();
            for line in TL
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with("//"))
            {
                let Some((fields, family)) = line.strip_suffix(';').and_then(|l| l.split_once('='))
                else {
                    continue;
                };
                let mut words = fields.split_whitespace();
                let Some((name, code)) = words.next().and_then(|decl| decl.split_once('#')) else {
                    continue;
                };
                let id = u32::from_str_radix(code, 16)
                    .map_err(|_| "ValueError: invalid schema constructor")?;
                let mut declarations = Vec::new();
                for word in words {
                    let (field, full) = word
                        .split_once(':')
                        .ok_or("ValueError: invalid schema field")?;
                    let (kind, flag) = if let Some((flag, kind)) = full.split_once('?') {
                        let (name, bit) = flag
                            .split_once('.')
                            .ok_or("ValueError: invalid schema flag")?;
                        let mask = bit
                            .parse::<u32>()
                            .ok()
                            .and_then(|bit| 1_u32.checked_shl(bit))
                            .ok_or("ValueError: invalid schema flag bit")?;
                        (kind, Some((name, mask)))
                    } else {
                        (full, None)
                    };
                    declarations.push(Field {
                        name: field,
                        kind,
                        flag,
                    });
                }
                rows.push((id, name, code, family.trim(), declarations));
            }
            let last: BTreeMap<_, _> = rows.iter().map(|row| (row.1, row.0)).collect();
            Ok(rows
                .into_iter()
                .map(|(id, name, code, family, fields)| {
                    let mut chars = name.chars();
                    let stem = chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default();
                    let class = if id == 0xaa48327d {
                        "DecryptedMessageService8".into()
                    } else if last.get(name) == Some(&id) {
                        stem
                    } else {
                        format!("{stem}_{code}")
                    };
                    Ctor {
                        id,
                        name,
                        family,
                        class,
                        fields,
                    }
                })
                .collect())
        })
        .as_deref()
        .map_err(Clone::clone)
}

fn find(id: u32) -> Outcome<&'static Ctor> {
    constructors()?
        .iter()
        .find(|ctor| ctor.id == id)
        .ok_or_else(|| format!("ValueError: UnknownConstructor: unknown constructor 0x{id:08x}"))
}

pub fn registry() -> Outcome<Vec<Value>> {
    Ok(constructors()?
        .iter()
        .map(|c| json!({"id":c.id,"name":c.name,"family":c.family,"class":c.class}))
        .collect())
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_decode(s: &str) -> Outcome<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut input = s.bytes().enumerate();
    while let Some((index, high)) = input.next() {
        if matches!(high, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            continue;
        }
        let invalid = |i| {
            format!("ValueError: non-hexadecimal number found in fromhex() arg at position {i}")
        };
        let high = hex_digit(high).ok_or_else(|| invalid(index))?;
        let (index, low) = input.next().ok_or_else(|| invalid(index + 1))?;
        let low = hex_digit(low).ok_or_else(|| invalid(index))?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_encode(raw: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(raw.len() * 2);
    for &b in raw {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

struct Reader<'a> {
    raw: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn block<const N: usize>(&mut self) -> Outcome<[u8; N]> {
        let offset = self.position;
        let end = offset + N;
        let raw = self.raw.get(offset..end).ok_or_else(|| format!("error: unpack_from requires a buffer of at least {end} bytes for unpacking {N} bytes at offset {offset} (actual buffer size is {})", self.raw.len()))?;
        self.position = end;
        raw.try_into()
            .map_err(|_| "ValueError: invalid binary field".into())
    }

    fn number(&mut self) -> Outcome<u32> {
        self.block().map(u32::from_le_bytes)
    }

    fn bytes(&mut self) -> Outcome<&[u8]> {
        let [first] = self.block()?;
        if first == 255 {
            return Err("ValueError: reserved TL bytes length prefix".into());
        }
        let (count, header) = if first == 254 {
            let [lo] = self.block()?;
            let [mid] = self.block()?;
            let [hi] = self.block()?;
            (u32::from_le_bytes([lo, mid, hi, 0]) as usize, 4)
        } else {
            (usize::from(first), 1)
        };
        if first == 254 && count < 254 {
            return Err("ValueError: noncanonical TL bytes length".into());
        }
        let end = self.position + count;
        let stop = end + ((4 - ((count + header) % 4)) % 4);
        if stop > self.raw.len() {
            return Err("ValueError: BufferError".into());
        }
        if self.raw[end..stop].iter().any(|&byte| byte != 0) {
            return Err("ValueError: nonzero TL bytes padding".into());
        }
        let start = self.position;
        self.position = stop;
        Ok(&self.raw[start..end])
    }

    fn object(&mut self, family: &str, depth: i64) -> Outcome<Value> {
        let id = self.number()?;
        let c = find(id)?;
        if !family.is_empty() && c.family != family {
            return Err("ValueError: unexpected TL result family".into());
        }
        if depth >= 32 {
            return Err("ValueError: nesting too deep".into());
        }
        let mut flags = BTreeMap::new();
        let mut fields = Map::new();
        for field in &c.fields {
            if field.kind == "#" {
                flags.insert(field.name, self.number()?);
                continue;
            }
            let present = field
                .flag
                .is_none_or(|(name, bit)| flags.get(name).is_some_and(|value| value & bit != 0));
            let data = if field.kind == "true" {
                Value::Bool(present)
            } else if !present {
                Value::Null
            } else {
                self.field(field.kind, depth + 1)?
            };
            fields.insert(field.name.to_owned(), data);
        }
        Ok(json!({"id":id,"fields":fields}))
    }

    fn field(&mut self, kind: &str, depth: i64) -> Outcome<Value> {
        match kind {
            "int" => Ok(json!(i32::from_le_bytes(self.block()?))),
            "long" => Ok(json!(i64::from_le_bytes(self.block()?))),
            "double" => {
                let value = f64::from_le_bytes(self.block()?);
                if value.is_finite() {
                    Ok(json!(value))
                } else {
                    Ok(
                        json!({"$num": if value.is_nan() {"NaN"} else if value.is_sign_negative() {"-Infinity"} else {"Infinity"}}),
                    )
                }
            }
            "Bool" => match self.number()? {
                BOOL_TRUE => Ok(Value::Bool(true)),
                BOOL_FALSE => Ok(Value::Bool(false)),
                id => Err(format!("RuntimeError: Invalid boolean code 0x{id:x}")),
            },
            "bytes" => Ok(json!({"hex":hex_encode(self.bytes()?)})),
            "string" => std::str::from_utf8(self.bytes()?)
                .map(|s| json!(s))
                .map_err(|_| "ValueError: UnicodeDecodeError".into()),
            _ if kind.starts_with("Vector<") => {
                if self.number()? != VECTOR {
                    return Err("ValueError: expected a vector".into());
                }
                let count = i32::from_le_bytes(self.block()?);
                let count = usize::try_from(count)
                    .ok()
                    .filter(|n| *n <= self.raw.len().saturating_sub(self.position) / 4)
                    .ok_or("ValueError: invalid vector length")?;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.field(&kind[7..kind.len() - 1], depth)?);
                }
                Ok(Value::Array(items))
            }
            _ => self.object(kind, depth),
        }
    }
}

fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

fn object_parts(value: &Value) -> Outcome<(&'static Ctor, &Map<String, Value>)> {
    let id = value.get("id").ok_or("KeyError: id")?;
    let id = id
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .ok_or("ValueError: invalid constructor id")?;
    let c = find(id)?;
    let fields = value
        .get("fields")
        .and_then(Value::as_object)
        .ok_or("TypeError: constructor fields must be an object")?;
    Ok((c, fields))
}

fn field_value<'a>(fields: &'a Map<String, Value>, field: &Field) -> &'a Value {
    fields.get(field.name).unwrap_or(&Value::Null)
}

fn encode_box(value: &Value, out: &mut Vec<u8>, depth: usize) -> Outcome<()> {
    if depth >= 32 {
        return Err("ValueError: nesting too deep".into());
    }
    let (c, fields) = object_parts(value)?;
    out.extend(c.id.to_le_bytes());
    for field in &c.fields {
        if field.kind == "#" {
            let mut bits = 0;
            for item in &c.fields {
                let Some((name, flag)) = item.flag else {
                    continue;
                };
                let data = field_value(fields, item);
                if name == field.name
                    && (if item.kind == "true" {
                        truth(data)
                    } else {
                        !data.is_null()
                    })
                {
                    bits |= flag;
                }
            }
            out.extend(bits.to_le_bytes());
        } else if field.kind != "true" {
            let data = field_value(fields, field);
            if field.flag.is_none() || !data.is_null() {
                encode_field(field.kind, data, out, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn pack_integer<const N: usize>(data: &Value, out: &mut Vec<u8>) -> Outcome<()> {
    let number = data
        .as_i64()
        .map(i128::from)
        .or_else(|| data.as_u64().map(i128::from))
        .or_else(|| data.as_bool().map(i128::from))
        .or_else(|| {
            data.get("$int")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
        })
        .ok_or("TypeError: '<=' not supported between instances of 'int' and 'NoneType'")?;
    let shift = N * 8;
    if number < -(1_i128 << (shift - 1)) || number >= 1_i128 << shift {
        return Err("error: argument out of range".into());
    }
    out.extend_from_slice(&number.to_le_bytes()[..N]);
    Ok(())
}

fn encode_field(kind: &str, data: &Value, out: &mut Vec<u8>, depth: usize) -> Outcome<()> {
    match kind {
        "int" => pack_integer::<4>(data, out)?,
        "long" => pack_integer::<8>(data, out)?,
        "double" => {
            let number = data
                .as_f64()
                .or_else(|| data.as_bool().map(|v| f64::from(u32::from(v))))
                .or_else(|| match data.get("$num").and_then(Value::as_str) {
                    Some("NaN") => Some(f64::NAN),
                    Some("Infinity") => Some(f64::INFINITY),
                    Some("-Infinity") => Some(f64::NEG_INFINITY),
                    Some("-0") => Some(-0.0),
                    _ => None,
                })
                .ok_or("error: required argument is not a float")?;
            out.extend(number.to_le_bytes());
        }
        "Bool" => out.extend((if truth(data) { BOOL_TRUE } else { BOOL_FALSE }).to_le_bytes()),
        "string" | "bytes" => {
            let owned;
            let raw = if let Some(s) = data.as_str() {
                s.as_bytes()
            } else if let Some(h) = data.get("hex").and_then(Value::as_str) {
                owned = hex_decode(h)?;
                &owned
            } else {
                return Err("TypeError: a TL string or bytes field takes str or bytes".into());
            };
            let count = raw.len();
            let start = out.len();
            if count < 254 {
                out.push(u8::try_from(count).map_err(|_| "OverflowError: int too big to convert")?);
            } else {
                if count > 0x00ff_ffff {
                    return Err("OverflowError: int too big to convert".into());
                }
                let count =
                    u32::try_from(count).map_err(|_| "OverflowError: int too big to convert")?;
                out.push(254);
                out.extend_from_slice(&count.to_le_bytes()[..3]);
            }
            out.extend_from_slice(raw);
            out.resize(out.len() + (4 - (out.len() - start) % 4) % 4, 0);
        }
        _ if kind.starts_with("Vector<") => {
            let items = data
                .as_array()
                .ok_or("TypeError: object of type 'NoneType' has no len()")?;
            let len = u32::try_from(items.len()).map_err(|_| "error: argument out of range")?;
            out.extend(VECTOR.to_le_bytes());
            out.extend(len.to_le_bytes());
            for item in items {
                encode_field(&kind[7..kind.len() - 1], item, out, depth)?;
            }
        }
        _ => {
            if object_parts(data)?.0.family != kind {
                return Err("ValueError: unexpected TL result family".into());
            }
            encode_box(data, out, depth)?;
        }
    }
    Ok(())
}

pub fn encode_object(constructor: Value) -> Outcome<String> {
    let raw = if constructor.is_object() {
        if let Some(hex) = constructor
            .get("hex")
            .and_then(Value::as_str)
            .filter(|_| constructor.as_object().is_some_and(|obj| obj.len() == 1))
        {
            hex_decode(hex)?
        } else {
            let mut raw = Vec::new();
            encode_box(&constructor, &mut raw, 0)?;
            raw
        }
    } else if let Some(items) = constructor.as_array() {
        items
            .iter()
            .map(|item| {
                if item.is_string() {
                    return Err(
                        "TypeError: 'str' object cannot be interpreted as an integer".into(),
                    );
                }
                item.as_i64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| "ValueError: bytes must be in range(0, 256)".to_owned())
            })
            .collect::<Outcome<Vec<_>>>()?
    } else if let Some(size) = constructor.as_i64() {
        let size = usize::try_from(size).map_err(|_| "ValueError: negative count")?;
        if size > 0x00ff_ffff {
            return Err("MemoryError: ".into());
        }
        let mut raw = Vec::new();
        raw.try_reserve_exact(size).map_err(|_| "MemoryError: ")?;
        raw.resize(size, 0);
        raw
    } else if constructor.is_string() {
        return Err("TypeError: string argument without an encoding".into());
    } else if constructor.is_null() {
        return Err("TypeError: cannot convert 'NoneType' object to bytes".into());
    } else {
        return Err("TypeError: cannot convert object to bytes".into());
    };
    Ok(hex_encode(&raw))
}

pub fn decode_object(raw: String, expected: String, depth: i64) -> Outcome<Value> {
    if depth < 0 {
        return Err("ValueError: invalid initial nesting depth".into());
    }
    let data = hex_decode(&raw)?;
    let mut reader = Reader {
        raw: &data,
        position: 0,
    };
    let object = reader.object(&expected, depth)?;
    let (c, fields) = object_parts(&object)?;
    let encoded = encode_object(object.clone())?;
    Ok(
        json!({"object":object,"position":reader.position,"encoded":encoded,"repr":format!("{}(<{} fields>)", c.class, fields.len()),"tag":c.name}),
    )
}

fn equivalent(left: &Value, right: &Value) -> Outcome<bool> {
    if left.get("id").is_some() || right.get("id").is_some() {
        if left.get("id") != right.get("id") {
            return Ok(false);
        }
        let (c, lf) = object_parts(left)?;
        let (_, rf) = object_parts(right)?;
        for f in c.fields.iter().filter(|f| f.kind != "#") {
            if !equivalent(field_value(lf, f), field_value(rf, f))? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if let (Some(l), Some(r)) = (left.as_array(), right.as_array()) {
        if l.len() != r.len() {
            return Ok(false);
        }
        for (l, r) in l.iter().zip(r) {
            if !equivalent(l, r)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if let (Some(l), Some(r)) = (
        left.get("hex").and_then(Value::as_str),
        right.get("hex").and_then(Value::as_str),
    ) {
        return Ok(hex_decode(l)? == hex_decode(r)?);
    }
    Ok(left == right)
}

fn validate_record_value(value: &Value) -> Outcome<()> {
    match value {
        Value::Object(o) if o.len() == 1 && o.contains_key("hex") => {
            hex_decode(
                o.get("hex")
                    .and_then(Value::as_str)
                    .ok_or("TypeError: fromhex() argument must be str")?,
            )?;
        }
        Value::Object(_) => {
            let (_, fields) = object_parts(value)?;
            for field in fields.values() {
                validate_record_value(field)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                validate_record_value(item)?;
            }
        }
        _ => (),
    }
    Ok(())
}

pub fn compare_objects(left: Value, right: Value) -> Outcome<bool> {
    validate_record_value(&left)?;
    validate_record_value(&right)?;
    equivalent(&left, &right)
}

pub fn abstract_object(_write: bool) -> Outcome<()> {
    Err("NotImplementedError: ".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_published_count_and_legacy_alias() -> Outcome<()> {
        let all = registry()?;
        assert_eq!(all.len(), 90);
        assert_eq!(all[1]["class"], "DecryptedMessageService8");
        Ok(())
    }

    #[test]
    fn validates_constructor_before_accepting_result_family() {
        assert_eq!(
            decode_object("deadbeef".into(), "DecryptedMessage".into(), 0),
            Err("ValueError: UnknownConstructor: unknown constructor 0xefbeadde".into())
        );
        assert_eq!(
            decode_object("4a5c9f08".into(), "DecryptedMessage".into(), 0),
            Err("ValueError: unexpected TL result family".into())
        );
    }

    #[test]
    fn refuses_depth_overflow_and_impossible_vectors() {
        let prefix = "634744f715c4b51c01000000";
        assert!(decode_object(format!("{}687b6d3f", prefix.repeat(31)), String::new(), 0).is_ok());
        assert_eq!(
            decode_object(format!("{}687b6d3f", prefix.repeat(32)), String::new(), 0),
            Err("ValueError: nesting too deep".into())
        );
        assert_eq!(
            decode_object("634744f715c4b51cffffffff".into(), String::new(), 0),
            Err("ValueError: invalid vector length".into())
        );
    }

    #[test]
    fn refuses_non_utf8_strings_without_disclosing_value() {
        assert_eq!(
            decode_object("7a761eb701ff0000".into(), String::new(), 0),
            Err("ValueError: UnicodeDecodeError".into())
        );
    }
}
