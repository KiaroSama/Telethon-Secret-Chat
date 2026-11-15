//! Bounded validation before grammers deserialization. In particular, grammers'
//! general-purpose String decoder replaces invalid UTF-8; secret chats refuse it.
use crate::{
    crypto::MAX_FRAME_LENGTH,
    error::{Error, Result},
};
use std::{collections::BTreeMap, sync::OnceLock};

struct Definition {
    family: String,
    fields: Vec<(String, String)>,
}
fn definitions() -> &'static BTreeMap<u32, Definition> {
    static DEFINITIONS: OnceLock<BTreeMap<u32, Definition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        include_str!("../schema/end-to-end.tl")
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with("//") || !line.ends_with(';') {
                    return None;
                }
                let (body, family) = line.trim_end_matches(';').split_once(" = ")?;
                let mut words = body.split_whitespace();
                let (_, id) = words.next()?.split_once('#')?;
                let fields = words
                    .filter_map(|w| w.split_once(':'))
                    .map(|(a, b)| (a.to_owned(), b.to_owned()))
                    .collect();
                Some((
                    u32::from_str_radix(id, 16).ok()?,
                    Definition {
                        family: family.to_owned(),
                        fields,
                    },
                ))
            })
            .collect()
    })
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    objects: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::WrapperRejected)?;
        let value = self
            .bytes
            .get(self.pos..end)
            .ok_or(Error::WrapperRejected)?;
        self.pos = end;
        Ok(value)
    }
    fn number(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| Error::WrapperRejected)?,
        ))
    }
    fn bytes(&mut self, string: bool) -> Result<()> {
        let first = self.take(1)?[0];
        let (len, header) = match first {
            255 => return Err(Error::WrapperRejected),
            254 => {
                let bytes = self.take(3)?;
                (
                    usize::from(bytes[0])
                        | (usize::from(bytes[1]) << 8)
                        | (usize::from(bytes[2]) << 16),
                    4,
                )
            }
            n => (usize::from(n), 1),
        };
        if header == 4 && len < 254 {
            return Err(Error::WrapperRejected);
        }
        let data = self.take(len)?;
        if string {
            std::str::from_utf8(data).map_err(|_| Error::WrapperRejected)?;
        }
        self.take((4 - (header + len) % 4) % 4)?;
        Ok(())
    }
    fn field(&mut self, ty: &str, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(Error::WrapperRejected);
        }
        match ty {
            "int" | "#" => {
                self.take(4)?;
            }
            "long" | "double" => {
                self.take(8)?;
            }
            "bytes" => self.bytes(false)?,
            "string" => self.bytes(true)?,
            "true" => {}
            "Bool" => {
                if !matches!(self.number()?, 0x997275b5 | 0xbc799737) {
                    return Err(Error::WrapperRejected);
                }
            }
            _ => {
                if let Some(inner) = ty.strip_prefix("Vector<").and_then(|s| s.strip_suffix('>')) {
                    if self.number()? != 0x1cb5c415 {
                        return Err(Error::WrapperRejected);
                    }
                    let len = self.number()? as usize;
                    if len > (self.bytes.len() - self.pos) / 4 {
                        return Err(Error::WrapperRejected);
                    }
                    for _ in 0..len {
                        self.field(inner, depth + 1)?;
                    }
                } else {
                    self.object(ty, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    fn object(&mut self, family: &str, depth: usize) -> Result<()> {
        self.objects += 1;
        if depth > 64 || self.objects > 65536 {
            return Err(Error::WrapperRejected);
        }
        let id = self.number()?;
        let definition = definitions().get(&id).ok_or(Error::WrapperRejected)?;
        if definition.family != family {
            return Err(Error::WrapperRejected);
        }
        let mut flags = BTreeMap::new();
        for (name, ty) in &definition.fields {
            if ty == "#" {
                flags.insert(name.as_str(), self.number()?);
                continue;
            }
            if let Some((condition, inner)) = ty.split_once('?') {
                let (flag, bit) = condition.split_once('.').ok_or(Error::WrapperRejected)?;
                let bit = bit.parse::<u32>().map_err(|_| Error::WrapperRejected)?;
                let mask = 1u32.checked_shl(bit).ok_or(Error::WrapperRejected)?;
                if flags.get(flag).copied().ok_or(Error::WrapperRejected)? & mask == 0 {
                    continue;
                }
                self.field(inner, depth + 1)?;
            } else {
                self.field(ty, depth + 1)?;
            }
        }
        Ok(())
    }
}
pub fn validate(bytes: &[u8], family: &str) -> Result<()> {
    if bytes.len() > MAX_FRAME_LENGTH {
        return Err(Error::WrapperRejected);
    }
    let mut reader = Reader {
        bytes,
        pos: 0,
        objects: 0,
    };
    reader.object(family, 0)?;
    if reader.pos != bytes.len() {
        return Err(Error::WrapperRejected);
    }
    Ok(())
}
pub fn constructor_count() -> usize {
    definitions().len()
}
