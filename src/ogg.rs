//! Bounded Ogg/Vorbis-comment classification shared with the consumer contract.
pub const HEADER_BYTES: usize = 64 * 1024;
const MUSIC_TAGS: [&[u8]; 8] = [
    b"TITLE",
    b"ARTIST",
    b"ALBUM",
    b"ALBUMARTIST",
    b"TRACKNUMBER",
    b"GENRE",
    b"PERFORMER",
    b"COMPOSER",
];
pub fn looks_like_voice(header: &[u8]) -> Option<bool> {
    let data = &header[..header.len().min(HEADER_BYTES)];
    let mut packets = Vec::new();
    let mut current = Vec::new();
    let mut serial = None;
    let mut position: usize = 0;
    while packets.len() < 2 {
        let page = data.get(position..position.checked_add(27)?)?;
        if &page[..4] != b"OggS" || page[4] != 0 {
            return None;
        }
        let flags = page[5];
        let count = usize::from(page[26]);
        let lacing = data.get(position + 27..position + 27 + count)?;
        let mut body = position + 27 + count;
        let end = body + lacing.iter().map(|n| usize::from(*n)).sum::<usize>();
        if end > data.len() {
            return None;
        }
        if serial.is_none() {
            if flags & 2 == 0 {
                return None;
            }
            serial = Some(page[14..18].to_vec());
        }
        if serial.as_deref() == Some(&page[14..18]) {
            if (flags & 1 != 0) != !current.is_empty() {
                return None;
            }
            for size in lacing {
                let size = usize::from(*size);
                current.extend_from_slice(&data[body..body + size]);
                body += size;
                if size < 255 {
                    packets.push(std::mem::take(&mut current));
                    if packets.len() == 2 {
                        break;
                    }
                }
            }
        }
        position = end;
    }
    let magic = if packets[0].starts_with(b"OpusHead") {
        b"OpusTags".as_slice()
    } else if packets[0].starts_with(b"\x01vorbis") {
        b"\x03vorbis".as_slice()
    } else {
        return None;
    };
    let block = packets[1].strip_prefix(magic)?;
    let mut cursor = 0;
    field(block, &mut cursor)?;
    let count = number(block, &mut cursor)? as usize;
    if count > (block.len() - cursor) / 4 {
        return None;
    }
    let mut music = false;
    for _ in 0..count {
        let tag = field(block, &mut cursor)?;
        if let Some(equal) = tag.iter().position(|b| *b == b'=') {
            music |= MUSIC_TAGS
                .iter()
                .any(|name| tag[..equal].eq_ignore_ascii_case(name));
        }
    }
    Some(!music)
}
fn number(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let end = cursor.checked_add(4)?;
    let n = u32::from_le_bytes(bytes.get(*cursor..end)?.try_into().ok()?);
    *cursor = end;
    Some(n)
}
fn field<'a>(bytes: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    let count = number(bytes, cursor)? as usize;
    let end = cursor.checked_add(count)?;
    let data = bytes.get(*cursor..end)?;
    *cursor = end;
    Some(data)
}
