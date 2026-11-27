use kiaro_secret_chat::protocol::key_visualization;
use kiaro_secret_chat::public_api::{CAPTIONLESS_KINDS, MEDIA_KINDS, PALETTE};

#[test]
fn official_key_picture_uses_blue_for_fourth_palette_index() {
    let picture = key_visualization(&[0xff; 36]).unwrap();
    let index = picture["rows"][0][0].as_u64().unwrap() as usize;
    assert_eq!(PALETTE[index], (47, 153, 201));
}

#[test]
fn baseline_media_vocabulary_keeps_video_and_voice_notes_distinct() {
    for kind in [
        "photo",
        "video",
        "document",
        "audio",
        "animation",
        "sticker",
        "video_note",
        "voice_note",
    ] {
        assert!(
            MEDIA_KINDS.contains(&kind),
            "missing baseline media kind: {kind}"
        );
    }
    assert!(CAPTIONLESS_KINDS.contains(&"video_note"));
    assert!(CAPTIONLESS_KINDS.contains(&"sticker"));
    assert!(!CAPTIONLESS_KINDS.contains(&"voice_note"));
}
