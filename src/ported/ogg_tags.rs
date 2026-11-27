//! Classify voice notes from the first Ogg comment packet.

use serde_json::{Value, json};

pub fn looks_like_voice(header: Vec<u8>) -> Result<Value, String> {
    Ok(json!(classify(&header[..header.len().min(64 * 1024)])))
}

fn classify(data: &[u8]) -> Option<bool> {
    let [ident, comment] = first_packets(data)?;
    let magic: &[u8] = if ident.starts_with(b"OpusHead") {
        b"OpusTags"
    } else if ident.starts_with(b"\x01vorbis") {
        b"\x03vorbis"
    } else {
        return None;
    };
    no_music_tags(comment.strip_prefix(magic)?)
}

fn first_packets(mut data: &[u8]) -> Option<[Vec<u8>; 2]> {
    let mut packets = Vec::with_capacity(2);
    let mut packet = Vec::new();
    let mut serial = None;
    while packets.len() < 2 {
        let (header, rest) = data.split_at_checked(27)?;
        if &header[..5] != b"OggS\0" {
            return None;
        }
        let flags = header[5];
        let (lacing, rest) = rest.split_at_checked(usize::from(header[26]))?;
        let size = lacing.iter().map(|x| usize::from(*x)).sum();
        let (mut body, next) = rest.split_at_checked(size)?;
        data = next;
        let page_serial = &header[14..18];
        if serial.is_none() {
            if flags & 2 == 0 {
                return None;
            }
            serial = Some(page_serial);
        }
        if serial != Some(page_serial) {
            continue;
        }
        if (flags & 1 != 0) == packet.is_empty() {
            return None;
        }
        for &size in lacing {
            let (segment, next) = body.split_at_checked(usize::from(size))?;
            body = next;
            packet.extend_from_slice(segment);
            if size < 255 {
                packets.push(std::mem::take(&mut packet));
                if packets.len() == 2 {
                    break;
                }
            }
        }
    }
    packets.try_into().ok()
}

fn uint32(input: &mut &[u8]) -> Option<usize> {
    let (bytes, rest) = input.split_at_checked(4)?;
    *input = rest;
    usize::try_from(u32::from_le_bytes(bytes.try_into().ok()?)).ok()
}

fn field<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = uint32(input)?;
    let (field, rest) = input.split_at_checked(len)?;
    *input = rest;
    Some(field)
}

fn no_music_tags(mut input: &[u8]) -> Option<bool> {
    const MUSIC: [&[u8]; 8] = [
        b"TITLE",
        b"ARTIST",
        b"ALBUM",
        b"ALBUMARTIST",
        b"TRACKNUMBER",
        b"GENRE",
        b"PERFORMER",
        b"COMPOSER",
    ];
    field(&mut input)?;
    let count = uint32(&mut input)?;
    if count > input.len() / 4 {
        return None;
    }
    let mut music = false;
    for _ in 0..count {
        let comment = field(&mut input)?;
        if let Some(eq) = comment.iter().position(|&b| b == b'=') {
            music |= MUSIC
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case(&comment[..eq]));
        }
    }
    Some(!music)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(tag: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 0, 1, 0, 0, 0];
        bytes.extend(u32::try_from(tag.len()).unwrap_or_default().to_le_bytes());
        bytes.extend(tag);
        bytes
    }

    #[test]
    fn tag_names_have_to_be_complete() {
        assert_eq!(no_music_tags(&comment(b"encoder=TITLE")), Some(true));
        assert_eq!(no_music_tags(&comment(b"artist=")), Some(false));
        assert_eq!(no_music_tags(&comment(b"TITLE")[..12]), None);
    }

    #[test]
    fn truncated_page_does_not_prove_voice() {
        assert_eq!(classify(b"OggS\0\x02"), None);
    }
}
