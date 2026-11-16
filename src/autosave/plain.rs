//! A schema-driven JSON view used for saved records, never for protocol execution.
use crate::error::{Error, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use grammers_tl_types::{Deserializable, deserialize::Cursor};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::OnceLock};
struct Definition {
    name: String,
    fields: Vec<(String, String)>,
}
fn definitions() -> &'static BTreeMap<u32, Definition> {
    static ALL: OnceLock<BTreeMap<u32, Definition>> = OnceLock::new();
    ALL.get_or_init(|| {
        include_str!("../../schema/end-to-end.tl")
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with("//") || !line.ends_with(';') {
                    return None;
                }
                let (body, _) = line[..line.len() - 1].split_once(" = ")?;
                let mut words = body.split_whitespace();
                let (name, id) = words.next()?.split_once('#')?;
                Some((
                    u32::from_str_radix(id, 16).ok()?,
                    Definition {
                        name: name.to_owned(),
                        fields: words
                            .filter_map(|word| word.split_once(':'))
                            .map(|(a, b)| (a.to_owned(), b.to_owned()))
                            .collect(),
                    },
                ))
            })
            .collect()
    })
}
pub fn plain_object(bytes: &[u8], family: &str) -> Result<Value> {
    crate::wire::validate(bytes, family)?;
    let mut cursor = Cursor::from_slice(bytes);
    let value = object(&mut cursor, 0)?;
    if cursor.pos() != bytes.len() {
        return Err(Error::Media);
    }
    Ok(value)
}
fn read<T: Deserializable>(cursor: &mut Cursor<'_>) -> Result<T> {
    T::deserialize(cursor).map_err(|_| Error::Media)
}
fn object(cursor: &mut Cursor<'_>, depth: usize) -> Result<Value> {
    if depth > 64 {
        return Err(Error::Media);
    }
    let id = read::<u32>(cursor)?;
    let definition = definitions().get(&id).ok_or(Error::Media)?;
    let mut object = Map::new();
    object.insert("_".into(), definition.name.clone().into());
    let mut flags = BTreeMap::new();
    for (name, ty) in &definition.fields {
        if ty == "#" {
            flags.insert(name.as_str(), read::<u32>(cursor)?);
            continue;
        }
        let value = if let Some((condition, inner)) = ty.split_once('?') {
            let (flag, bit) = condition.split_once('.').ok_or(Error::Media)?;
            let bit = bit.parse::<u32>().map_err(|_| Error::Media)?;
            let present = flags.get(flag).copied().ok_or(Error::Media)?
                & 1u32.checked_shl(bit).ok_or(Error::Media)?
                != 0;
            if inner == "true" {
                Value::Bool(present)
            } else if present {
                field(cursor, inner, depth + 1)?
            } else {
                Value::Null
            }
        } else {
            field(cursor, ty, depth + 1)?
        };
        object.insert(name.clone(), value);
    }
    Ok(Value::Object(object))
}
fn field(cursor: &mut Cursor<'_>, ty: &str, depth: usize) -> Result<Value> {
    if depth > 64 {
        return Err(Error::Media);
    }
    Ok(match ty {
        "int" => Value::from(read::<i32>(cursor)?),
        "long" => Value::from(read::<i64>(cursor)?),
        "double" => serde_json::Number::from_f64(read::<f64>(cursor)?)
            .map(Value::Number)
            .ok_or(Error::Media)?,
        "string" => Value::from(read::<String>(cursor)?),
        "bytes" => Value::from(STANDARD.encode(read::<Vec<u8>>(cursor)?)),
        "Bool" => Value::Bool(read::<bool>(cursor)?),
        "true" => Value::Bool(true),
        _ => {
            if let Some(inner) = ty.strip_prefix("Vector<").and_then(|s| s.strip_suffix('>')) {
                if read::<u32>(cursor)? != 0x1cb5c415 {
                    return Err(Error::Media);
                }
                let count = read::<u32>(cursor)?;
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(field(cursor, inner, depth + 1)?);
                }
                Value::Array(values)
            } else {
                object(cursor, depth + 1)?
            }
        }
    })
}
