//! Exact Desktop static export assets stored as Rust data and PNG bytes.
use super::{error::Result, output_file::OutputFile};
use std::path::Path;
#[path = "assets/script_0.rs"]
mod script_0;
#[path = "assets/style_0.rs"]
mod style_0;
#[path = "assets/style_1.rs"]
mod style_1;
#[path = "assets/style_2.rs"]
mod style_2;
#[path = "assets/style_3.rs"]
mod style_3;
pub const ASSET_NAMES: &[&str] = &[
    "css/style.css",
    "images/back.png",
    "images/back@2x.png",
    "images/media_call.png",
    "images/media_call@2x.png",
    "images/media_contact.png",
    "images/media_contact@2x.png",
    "images/media_file.png",
    "images/media_file@2x.png",
    "images/media_game.png",
    "images/media_game@2x.png",
    "images/media_location.png",
    "images/media_location@2x.png",
    "images/media_music.png",
    "images/media_music@2x.png",
    "images/media_photo.png",
    "images/media_photo@2x.png",
    "images/media_shop.png",
    "images/media_shop@2x.png",
    "images/media_video.png",
    "images/media_video@2x.png",
    "images/media_voice.png",
    "images/media_voice@2x.png",
    "images/section_calls.png",
    "images/section_calls@2x.png",
    "images/section_chats.png",
    "images/section_chats@2x.png",
    "images/section_contacts.png",
    "images/section_contacts@2x.png",
    "images/section_frequent.png",
    "images/section_frequent@2x.png",
    "images/section_music.png",
    "images/section_music@2x.png",
    "images/section_other.png",
    "images/section_other@2x.png",
    "images/section_photos.png",
    "images/section_photos@2x.png",
    "images/section_sessions.png",
    "images/section_sessions@2x.png",
    "images/section_stories.png",
    "images/section_stories@2x.png",
    "images/section_web.png",
    "images/section_web@2x.png",
    "js/script.js",
];
// Original Desktop CSS/JS use CRLF. Rust string literals normalize newlines;
// restore the asset byte convention when materializing the output files.
pub fn asset_bytes(name: &str) -> Option<Vec<u8>> {
    match name {
        "css/style.css" => Some(
            [style_0::DATA, style_1::DATA, style_2::DATA, style_3::DATA]
                .concat()
                .replace('\n', "\r\n")
                .into_bytes(),
        ),
        "js/script.js" => Some([script_0::DATA].concat().replace('\n', "\r\n").into_bytes()),
        "images/back.png" => Some(include_bytes!("assets/images/back.png").to_vec()),
        "images/back@2x.png" => Some(include_bytes!("assets/images/back@2x.png").to_vec()),
        "images/media_call.png" => Some(include_bytes!("assets/images/media_call.png").to_vec()),
        "images/media_call@2x.png" => {
            Some(include_bytes!("assets/images/media_call@2x.png").to_vec())
        }
        "images/media_contact.png" => {
            Some(include_bytes!("assets/images/media_contact.png").to_vec())
        }
        "images/media_contact@2x.png" => {
            Some(include_bytes!("assets/images/media_contact@2x.png").to_vec())
        }
        "images/media_file.png" => Some(include_bytes!("assets/images/media_file.png").to_vec()),
        "images/media_file@2x.png" => {
            Some(include_bytes!("assets/images/media_file@2x.png").to_vec())
        }
        "images/media_game.png" => Some(include_bytes!("assets/images/media_game.png").to_vec()),
        "images/media_game@2x.png" => {
            Some(include_bytes!("assets/images/media_game@2x.png").to_vec())
        }
        "images/media_location.png" => {
            Some(include_bytes!("assets/images/media_location.png").to_vec())
        }
        "images/media_location@2x.png" => {
            Some(include_bytes!("assets/images/media_location@2x.png").to_vec())
        }
        "images/media_music.png" => Some(include_bytes!("assets/images/media_music.png").to_vec()),
        "images/media_music@2x.png" => {
            Some(include_bytes!("assets/images/media_music@2x.png").to_vec())
        }
        "images/media_photo.png" => Some(include_bytes!("assets/images/media_photo.png").to_vec()),
        "images/media_photo@2x.png" => {
            Some(include_bytes!("assets/images/media_photo@2x.png").to_vec())
        }
        "images/media_shop.png" => Some(include_bytes!("assets/images/media_shop.png").to_vec()),
        "images/media_shop@2x.png" => {
            Some(include_bytes!("assets/images/media_shop@2x.png").to_vec())
        }
        "images/media_video.png" => Some(include_bytes!("assets/images/media_video.png").to_vec()),
        "images/media_video@2x.png" => {
            Some(include_bytes!("assets/images/media_video@2x.png").to_vec())
        }
        "images/media_voice.png" => Some(include_bytes!("assets/images/media_voice.png").to_vec()),
        "images/media_voice@2x.png" => {
            Some(include_bytes!("assets/images/media_voice@2x.png").to_vec())
        }
        "images/section_calls.png" => {
            Some(include_bytes!("assets/images/section_calls.png").to_vec())
        }
        "images/section_calls@2x.png" => {
            Some(include_bytes!("assets/images/section_calls@2x.png").to_vec())
        }
        "images/section_chats.png" => {
            Some(include_bytes!("assets/images/section_chats.png").to_vec())
        }
        "images/section_chats@2x.png" => {
            Some(include_bytes!("assets/images/section_chats@2x.png").to_vec())
        }
        "images/section_contacts.png" => {
            Some(include_bytes!("assets/images/section_contacts.png").to_vec())
        }
        "images/section_contacts@2x.png" => {
            Some(include_bytes!("assets/images/section_contacts@2x.png").to_vec())
        }
        "images/section_frequent.png" => {
            Some(include_bytes!("assets/images/section_frequent.png").to_vec())
        }
        "images/section_frequent@2x.png" => {
            Some(include_bytes!("assets/images/section_frequent@2x.png").to_vec())
        }
        "images/section_music.png" => {
            Some(include_bytes!("assets/images/section_music.png").to_vec())
        }
        "images/section_music@2x.png" => {
            Some(include_bytes!("assets/images/section_music@2x.png").to_vec())
        }
        "images/section_other.png" => {
            Some(include_bytes!("assets/images/section_other.png").to_vec())
        }
        "images/section_other@2x.png" => {
            Some(include_bytes!("assets/images/section_other@2x.png").to_vec())
        }
        "images/section_photos.png" => {
            Some(include_bytes!("assets/images/section_photos.png").to_vec())
        }
        "images/section_photos@2x.png" => {
            Some(include_bytes!("assets/images/section_photos@2x.png").to_vec())
        }
        "images/section_sessions.png" => {
            Some(include_bytes!("assets/images/section_sessions.png").to_vec())
        }
        "images/section_sessions@2x.png" => {
            Some(include_bytes!("assets/images/section_sessions@2x.png").to_vec())
        }
        "images/section_stories.png" => {
            Some(include_bytes!("assets/images/section_stories.png").to_vec())
        }
        "images/section_stories@2x.png" => {
            Some(include_bytes!("assets/images/section_stories@2x.png").to_vec())
        }
        "images/section_web.png" => Some(include_bytes!("assets/images/section_web.png").to_vec()),
        "images/section_web@2x.png" => {
            Some(include_bytes!("assets/images/section_web@2x.png").to_vec())
        }
        _ => None,
    }
}
pub fn write_assets(folder: &Path) -> Result<()> {
    for name in ASSET_NAMES {
        let mut file = OutputFile::new(folder.join(name));
        if let Some(bytes) = asset_bytes(name) {
            file.write_block(&bytes)?;
        }
        file.close();
    }
    Ok(())
}
