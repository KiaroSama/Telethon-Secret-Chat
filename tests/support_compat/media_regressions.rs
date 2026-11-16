//! Public SDK media-wrapper regressions in the existing compatibility scenario.
#![allow(clippy::unwrap_used)]
use grammers_client::media::{Document, Downloadable, InputVideo, Photo, Sticker, Uploaded};
use grammers_tl_types::{enums as api, types as raw};
use serde_json::json;
fn doc() -> raw::MessageMediaDocument {
    serde_json::from_value(json!({"nopremium":false,"spoiler":true,"video":false,"round":false,"voice":false,"document":null,"alt_documents":null,"video_cover":null,"video_timestamp":null,"ttl_seconds":null})).unwrap()
}
pub fn run() {
    let photo = Photo::from_raw_media(raw::MessageMediaPhoto {
        spoiler: true,
        live_photo: false,
        photo: None,
        ttl_seconds: None,
        video: None,
    });
    assert_eq!(photo.id(), None);
    assert!(photo.to_raw_input_media().spoiler);
    let document = Document::from_raw_media(doc());
    assert_eq!(document.id(), None);
    assert_eq!(document.name(), None);
    assert!(document.to_raw_input_media().spoiler);
    let uploaded = Uploaded::from_raw(
        raw::InputFile {
            id: 1,
            parts: 1,
            name: "video.mp4".into(),
            md5_checksum: String::new(),
        }
        .into(),
    );
    let api::InputMedia::UploadedDocument(video) = InputVideo::new(uploaded.clone()).into() else {
        panic!("video conversion")
    };
    assert!(video.nosound_video);
    let api::InputMedia::UploadedDocument(animation) =
        InputVideo::new(uploaded).animated(true).into()
    else {
        panic!("animation conversion")
    };
    assert!(!animation.nosound_video);
    assert!(
        animation
            .attributes
            .iter()
            .any(|attribute| matches!(attribute, api::DocumentAttribute::Animated))
    );

    let sticker_attr = raw::DocumentAttributeSticker {
        mask: false,
        alt: "public".into(),
        stickerset: api::InputStickerSet::Empty,
        mask_coords: None,
    };
    for (mime, animated_attribute, expected) in [
        ("application/x-tgsticker", false, true),
        ("image/webp", true, false),
    ] {
        let mut media = doc();
        let mut attributes = vec![sticker_attr.clone().into()];
        if animated_attribute {
            attributes.push(api::DocumentAttribute::Animated);
        }
        media.document = Some(
            raw::Document {
                id: 7,
                access_hash: 1,
                file_reference: vec![],
                date: 0,
                mime_type: mime.into(),
                size: 1,
                thumbs: None,
                video_thumbs: None,
                dc_id: 2,
                attributes,
            }
            .into(),
        );
        let document = Document::from_raw_media(media);
        assert_eq!(document.id(), Some(7));
        assert_eq!(
            Sticker::from_document(&document).unwrap().is_animated(),
            expected
        );
    }
    let photo = Photo::from_raw(
        raw::Photo {
            has_stickers: false,
            id: 7,
            access_hash: 1,
            file_reference: vec![],
            date: 0,
            sizes: vec![
                raw::PhotoPathSize {
                    r#type: "j".into(),
                    bytes: vec![5, 170, 65],
                }
                .into(),
            ],
            video_sizes: None,
            dc_id: 2,
        }
        .into(),
    );
    assert_eq!(photo.id(), Some(7));
    let data = photo.thumbs()[0].to_data().unwrap();
    let svg = String::from_utf8(data).unwrap();
    assert!(svg.contains("d=\"M5,42-1z\""));
    assert!(
        !svg.chars()
            .any(|c| c < ' ' && !matches!(c, '\n' | '\r' | '\t'))
    );
}
