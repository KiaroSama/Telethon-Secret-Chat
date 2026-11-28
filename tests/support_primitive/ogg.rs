use kiaro_secret_chat::{
    media::{MediaKind, resolve_kind},
    ogg::looks_like_voice,
};
pub fn page(parts: &[(&[u8], bool)], flags: u8, serial: u32, sequence: u32) -> Vec<u8> {
    let mut lace = Vec::new();
    for (data, complete) in parts {
        assert!(*complete || data.len() % 255 == 0);
        lace.extend(std::iter::repeat_n(255u8, data.len() / 255));
        if *complete {
            lace.push((data.len() % 255) as u8);
        }
    }
    assert!(lace.len() <= 255);
    let mut result = b"OggS".to_vec();
    result.extend([0, flags]);
    result.extend([0; 8]);
    result.extend(serial.to_le_bytes());
    result.extend(sequence.to_le_bytes());
    result.extend([0; 4]);
    result.push(lace.len() as u8);
    result.extend(lace);
    for (data, _) in parts {
        result.extend_from_slice(data);
    }
    result
}
pub fn comment(comments: &[&str], vorbis: bool) -> Vec<u8> {
    let mut result = if vorbis {
        b"\x03vorbis".to_vec()
    } else {
        b"OpusTags".to_vec()
    };
    result.extend(4u32.to_le_bytes());
    result.extend_from_slice(b"test");
    result.extend((comments.len() as u32).to_le_bytes());
    for comment in comments {
        result.extend((comment.len() as u32).to_le_bytes());
        result.extend_from_slice(comment.as_bytes());
    }
    if vorbis {
        result.push(1);
    }
    result
}
fn ident(vorbis: bool) -> Vec<u8> {
    if vorbis {
        [b"\x01vorbis".as_slice(), &[0; 23]].concat()
    } else {
        [b"OpusHead".as_slice(), &[1, 1], &[0; 9]].concat()
    }
}
pub fn ogg(comments: &[&str], vorbis: bool, split: bool) -> Vec<u8> {
    let mut result = page(&[(&ident(vorbis), true)], 2, 1, 0);
    let packet = comment(comments, vorbis);
    if !split {
        result.extend(page(&[(&packet, true)], 0, 1, 1));
    } else {
        let cut = 255 * (packet.len() / 255 - usize::from(packet.len().is_multiple_of(255)));
        assert!(cut > 0 && cut < packet.len());
        result.extend(page(&[(&packet[..cut], false)], 0, 1, 1));
        result.extend(page(&[(&packet[cut..], true)], 1, 1, 2));
    }
    result
}
pub fn voice(case: &str) {
    match case {
        "recording" => assert_eq!(looks_like_voice(&ogg(&[], false, false)), Some(true)),
        "encoder" => assert_eq!(
            looks_like_voice(&ogg(&["ENCODER=Lavf62.3.100"], false, false)),
            Some(true)
        ),
        "not_ogg" => {
            assert_eq!(looks_like_voice(b"ID3 not an ogg"), None);
            assert_eq!(looks_like_voice(b""), None);
        }
        "tag_inside_value" => assert_eq!(
            looks_like_voice(&ogg(&["ENCODER=TITLEmaker 1.0"], false, false)),
            Some(true)
        ),
        "infer_voice" => {
            assert_eq!(
                resolve_kind(None, "audio/ogg", "", &ogg(&[], false, false)).unwrap(),
                MediaKind::VoiceNote
            );
            assert_eq!(
                resolve_kind(None, "audio/ogg", "", &[]).unwrap(),
                MediaKind::Audio
            );
        }
        "infer_music" => assert_eq!(
            resolve_kind(None, "audio/ogg", "", &ogg(&["TITLE=A Song"], false, false)).unwrap(),
            MediaKind::Audio
        ),
        "explicit_voice" => assert_eq!(
            resolve_kind(
                Some(MediaKind::VoiceNote),
                "audio/ogg",
                "",
                &ogg(&["TITLE=A Song"], false, false)
            )
            .unwrap(),
            MediaKind::VoiceNote
        ),
        "non_audio" => {
            for (mime, kind) in [
                ("image/jpeg", MediaKind::Photo),
                ("video/mp4", MediaKind::Video),
                ("image/gif", MediaKind::Animation),
            ] {
                assert_eq!(
                    resolve_kind(None, mime, "", &ogg(&[], false, false)).unwrap(),
                    kind
                );
            }
        }
        music if music.contains('=') => {
            assert_eq!(looks_like_voice(&ogg(&[music], false, false)), Some(false))
        }
        mime if mime.starts_with("audio/") => {
            if matches!(mime, "audio/ogg" | "audio/opus") {
                for kind in [MediaKind::VoiceNote, MediaKind::Audio] {
                    assert_eq!(resolve_kind(Some(kind), mime, "", &[]).unwrap(), kind);
                }
            } else {
                assert!(resolve_kind(Some(MediaKind::VoiceNote), mime, "", &[]).is_err());
            }
        }
        _ => panic!("unknown voice scenario"),
    }
}
pub fn audit(case: &str) {
    let pad = format!("ENCODER={}", "x".repeat(300));
    match case {
        "lowercase" => assert_eq!(
            looks_like_voice(&ogg(&["artist=somebody"], false, false)),
            Some(false)
        ),
        "magic_outside_comment" => {
            let stream = [
                page(&[(&ident(false), true)], 2, 1, 0),
                page(&[(b"audio frame", true)], 0, 1, 1),
                page(&[(&comment(&["TITLE=A Song"], false), true)], 0, 1, 2),
            ]
            .concat();
            assert_eq!(looks_like_voice(&stream), None);
        }
        "other_logical_stream" => {
            let stream = [
                page(&[(&ident(false), true)], 2, 1, 0),
                page(&[(&comment(&["TITLE=Other stream"], false), true)], 0, 2, 0),
                page(&[(&comment(&[], false), true)], 0, 1, 1),
            ]
            .concat();
            assert_eq!(looks_like_voice(&stream), Some(true));
        }
        "bad capture pattern" => {
            let mut stream = ogg(&["ENCODER=x"], false, false);
            stream[..4].copy_from_slice(b"OggX");
            assert_eq!(looks_like_voice(&stream), None);
        }
        "unknown stream structure version" => {
            let mut stream = ogg(&["ENCODER=x"], false, false);
            stream[4] = 1;
            assert_eq!(looks_like_voice(&stream), None);
        }
        "first page is not beginning of stream" => {
            let mut stream = ogg(&["ENCODER=x"], false, false);
            stream[5] = 0;
            assert_eq!(looks_like_voice(&stream), None);
        }
        "lacing runs past the buffer" => {
            let mut stream = ogg(&["ENCODER=x"], false, false);
            stream.truncate(stream.len() - 3);
            assert_eq!(looks_like_voice(&stream), None);
        }
        "packet unfinished at buffer end" => {
            let packet = comment(&[&pad], false);
            let stream = [
                page(&[(&ident(false), true)], 2, 1, 0),
                page(&[(&packet[..255], false)], 0, 1, 1),
            ]
            .concat();
            assert_eq!(looks_like_voice(&stream), None);
        }
        "continuation flag with nothing to continue" => {
            let stream = [
                page(&[(&ident(false), true)], 3, 1, 0),
                page(&[(&comment(&[], false), true)], 0, 1, 1),
            ]
            .concat();
            assert_eq!(looks_like_voice(&stream), None);
        }
        "first packet is not an identification header" => {
            let stream = [
                page(&[(b"garbage", true)], 2, 1, 0),
                page(&[(&comment(&[], false), true)], 0, 1, 1),
            ]
            .concat();
            assert_eq!(looks_like_voice(&stream), None);
        }
        text if text.starts_with("truncate_") => {
            let removed = text[9..].parse::<usize>().unwrap();
            let mut stream = ogg(&["ENCODER=test"], false, false);
            stream.truncate(stream.len() - removed);
            assert_eq!(looks_like_voice(&stream), None);
        }
        text if text.starts_with("split_") => {
            let vorbis = text.ends_with("vorbis");
            let music = text.contains("music");
            let comments = if music {
                vec![pad.as_str(), "TITLE=A Song"]
            } else {
                vec![pad.as_str()]
            };
            assert_eq!(
                looks_like_voice(&ogg(&comments, vorbis, true)),
                Some(!music)
            );
        }
        text if text.contains('=') => {
            assert_eq!(looks_like_voice(&ogg(&[text], false, false)), Some(true))
        }
        _ => panic!("unknown Ogg framing scenario"),
    }
}
