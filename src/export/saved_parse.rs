//! Secret autosave records mapped to the Desktop message model without consuming the archive.
use super::{
    error::Result,
    files::file_name_from_user_string,
    format::DateContext,
    model::*,
    model_actions::{
        Action, ActionHistoryClear, ActionScreenshotTaken, ActionSetMessagesTTL, ServiceAction,
    },
    model_document::{compute_document_name, document_folder, parse_attributes},
    model_media::{GeoPoint, Media, MediaContent, SharedContact, Venue},
    model_message::Message,
    model_peers::ContactInfo,
};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};
fn action(v: &Value) -> Option<Action> {
    match kind(v) {
        "decryptedMessageActionSetMessageTTL" => {
            Some(Action::SetMessagesTTL(ActionSetMessagesTTL {
                period: integer(v, "ttl_seconds"),
            }))
        }
        "decryptedMessageActionScreenshotMessages" => {
            Some(Action::ScreenshotTaken(ActionScreenshotTaken::default()))
        }
        "decryptedMessageActionFlushHistory" => {
            Some(Action::HistoryClear(ActionHistoryClear::default()))
        }
        _ => None,
    }
}
fn media(
    context: &mut ParseMediaContext,
    v: &Value,
    date: i64,
    dates: &DateContext,
) -> Result<Media> {
    let content = match kind(v) {
        "decryptedMessageMediaPhoto" => {
            context.photos += 1;
            Some(MediaContent::Photo(Photo {
                date,
                image: Image {
                    width: integer(v, "w").max(0) as u32,
                    height: integer(v, "h").max(0) as u32,
                    file: File {
                        size: integer(v, "size").max(0) as u64,
                        suggested_path: format!(
                            "photos/{}",
                            prepare_photo_file_name(context.photos, date, dates)?
                        ),
                        ..Default::default()
                    },
                },
                ..Default::default()
            }))
        }
        "decryptedMessageMediaDocument"
        | "decryptedMessageMediaExternalDocument"
        | "decryptedMessageMediaVideo"
        | "decryptedMessageMediaAudio" => {
            let mut d = Document {
                date,
                mime: string(v, "mime_type").into(),
                ..Default::default()
            };
            d.file.size = integer(v, "size").max(0) as u64;
            match kind(v) {
                "decryptedMessageMediaVideo" => {
                    if d.mime.is_empty() {
                        d.mime = "video/mp4".into();
                    }
                    d.is_video_file = true;
                    d.width = integer(v, "w").max(0) as u32;
                    d.height = integer(v, "h").max(0) as u32;
                    d.duration = integer(v, "duration").max(0) as u32;
                }
                "decryptedMessageMediaAudio" => {
                    if d.mime.is_empty() {
                        d.mime = "audio/ogg".into();
                    }
                    d.is_voice_message = true;
                    d.duration = integer(v, "duration").max(0) as u32;
                }
                _ => {
                    d.name = string(v, "file_name").into();
                    let attributes = array(v, "attributes")
                        .iter()
                        .filter(|a| kind(a) != "documentAttributeCustomEmoji")
                        .cloned()
                        .collect::<Vec<_>>();
                    parse_attributes(&mut d, &attributes);
                }
            }
            d.file.suggested_path = format!(
                "{}/{}",
                document_folder(&d),
                file_name_from_user_string(&compute_document_name(context, &d, date, dates)?)
            );
            Some(MediaContent::Document(d))
        }
        "decryptedMessageMediaGeoPoint" => Some(MediaContent::GeoPoint(point(v))),
        "decryptedMessageMediaVenue" => Some(MediaContent::Venue(Venue {
            point: point(v),
            title: string(v, "title").into(),
            address: string(v, "address").into(),
        })),
        "decryptedMessageMediaContact" => Some(MediaContent::SharedContact(SharedContact {
            info: ContactInfo {
                user_id: integer(v, "user_id"),
                first_name: string(v, "first_name").into(),
                last_name: string(v, "last_name").into(),
                phone_number: string(v, "phone_number").into(),
                ..Default::default()
            },
            ..Default::default()
        })),
        _ => None,
    };
    Ok(Media { content, ttl: 0 })
}
fn point(v: &Value) -> GeoPoint {
    GeoPoint {
        latitude: v["lat"].as_f64().unwrap_or(0.0),
        longitude: v["long"].as_f64().unwrap_or(0.0),
        valid: true,
    }
}
fn entity(v: &Value) -> Option<Value> {
    let name = kind(v);
    let suffix = name.strip_prefix("messageEntity")?;
    let extra = match suffix {
        "Unknown" | "Mention" | "Hashtag" | "BotCommand" | "Url" | "Email" | "Bold" | "Italic"
        | "Code" | "Phone" | "Cashtag" | "Underline" | "Strike" | "BankCard" | "Spoiler" => None,
        "Pre" => Some("language"),
        "TextUrl" => Some("url"),
        "MentionName" => Some("user_id"),
        "CustomEmoji" => Some("document_id"),
        "Blockquote" => None,
        _ => return None,
    };
    let fields = v.as_object()?;
    if !fields.contains_key("offset")
        || !fields.contains_key("length")
        || extra.is_some_and(|k| !fields.contains_key(k))
    {
        return None;
    }
    if fields.keys().any(|k| {
        !["_", "offset", "length"].contains(&k.as_str())
            && Some(k.as_str()) != extra
            && !(suffix == "Blockquote" && k == "collapsed")
    }) {
        return None;
    }
    Some(v.clone())
}
pub(crate) fn parse_with_sources(
    records: &[Value],
    own: &User,
    peer: &User,
    dates: &DateContext,
) -> Result<(Vec<Message>, BTreeMap<i64, PathBuf>)> {
    let mut context = ParseMediaContext {
        self_peer_id: peer_from_user(own.id),
        ..Default::default()
    };
    let mut kept = Vec::new();
    let mut ids = BTreeMap::new();
    for record in records {
        let mut message = match string(record, "type") {
            "message" => Message::default(),
            "service" => {
                let Some(content) = action(&record["action"]) else {
                    continue;
                };
                Message {
                    action: ServiceAction {
                        content: Some(content),
                    },
                    ..Default::default()
                }
            }
            _ => continue,
        };
        message.id = kept.len() as i64 + 1;
        message.date = integer(record, "date");
        message.out = boolean(record, "out");
        message.peer_id = peer_from_user(peer.id);
        message.self_id = context.self_peer_id;
        message.from_id = if message.out {
            message.self_id
        } else {
            message.peer_id
        };
        ids.insert(
            record.get("id").unwrap_or(&Value::Null).to_string(),
            message.id,
        );
        kept.push((record, message));
    }
    let mut sources = BTreeMap::new();
    let mut messages = Vec::with_capacity(kept.len());
    for (record, mut message) in kept {
        if string(record, "type") == "message" {
            let saved_media = &record["media"];
            message.media = media(&mut context, saved_media, message.date, dates)?;
            let text = string(record, "text");
            let text = if text.is_empty() {
                string(saved_media, "caption")
            } else {
                text
            };
            let entities = array(record, "entities")
                .iter()
                .filter_map(entity)
                .collect::<Vec<_>>();
            message.text = parse_text(text, &entities);
            for part in &mut message.text {
                if part.kind == TextType::CustomEmoji {
                    part.additional = TextPart::UNAVAILABLE_EMOJI.into();
                }
            }
            message.reply_to_msg_id = record
                .get("reply_to")
                .filter(|v| !v.is_null())
                .and_then(|r| ids.get(&r.to_string()))
                .copied()
                .unwrap_or(0);
            let source = string(record, "file");
            if !source.is_empty()
                && matches!(
                    message.media.content,
                    Some(MediaContent::Photo(_) | MediaContent::Document(_))
                )
            {
                sources.insert(message.id, PathBuf::from(source));
            }
        }
        messages.push(message);
    }
    Ok((messages, sources))
}
pub fn parse_saved_messages(
    records: &[Value],
    own: &User,
    peer: &User,
    dates: &DateContext,
) -> Result<Vec<Message>> {
    parse_with_sources(records, own, peer, dates).map(|(m, _)| m)
}
