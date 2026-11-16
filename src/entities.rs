//! Convert only entities representable by the negotiated secret-chat schema.
use crate::{enums::MessageEntity as Target, types};
use grammers_tl_types::enums::MessageEntity as Source;
pub fn convert(entities: Vec<Source>, layer: i32) -> Vec<Target> {
    entities
        .into_iter()
        .filter_map(|entity| match entity {
            Source::Unknown(e) => Some(
                types::MessageEntityUnknown {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Mention(e) => Some(
                types::MessageEntityMention {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Hashtag(e) => Some(
                types::MessageEntityHashtag {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::BotCommand(e) => Some(
                types::MessageEntityBotCommand {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Url(e) => Some(
                types::MessageEntityUrl {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Email(e) => Some(
                types::MessageEntityEmail {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Bold(e) => Some(
                types::MessageEntityBold {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Italic(e) => Some(
                types::MessageEntityItalic {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Code(e) => Some(
                types::MessageEntityCode {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Pre(e) => Some(
                types::MessageEntityPre {
                    offset: e.offset,
                    length: e.length,
                    language: e.language,
                }
                .into(),
            ),
            Source::TextUrl(e) => Some(
                types::MessageEntityTextUrl {
                    offset: e.offset,
                    length: e.length,
                    url: e.url,
                }
                .into(),
            ),
            Source::Underline(e) if layer >= 101 => Some(
                types::MessageEntityUnderline {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Strike(e) if layer >= 101 => Some(
                types::MessageEntityStrike {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Blockquote(e) if layer >= 101 => Some(
                types::MessageEntityBlockquote {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::Spoiler(e) if layer >= 144 => Some(
                types::MessageEntitySpoiler {
                    offset: e.offset,
                    length: e.length,
                }
                .into(),
            ),
            Source::CustomEmoji(e) if layer >= 144 => Some(
                types::MessageEntityCustomEmoji {
                    offset: e.offset,
                    length: e.length,
                    document_id: e.document_id,
                }
                .into(),
            ),
            _ => None,
        })
        .collect()
}

/// Accept both API entities and existing secret-schema entities. Secret entities
/// retain their wire values, subject to the same negotiated-layer restrictions.
pub enum EntityInput {
    Api(Source),
    Secret(Target),
}
pub fn convert_inputs(entities: Vec<EntityInput>, layer: i32) -> Vec<Target> {
    entities
        .into_iter()
        .flat_map(|entity| match entity {
            EntityInput::Api(value) => convert(vec![value], layer),
            EntityInput::Secret(value) => {
                let minimum = match value {
                    Target::Underline(_) | Target::Strike(_) | Target::Blockquote(_) => 101,
                    Target::Spoiler(_) | Target::CustomEmoji(_) => 144,
                    _ => 46,
                };
                if layer >= minimum {
                    vec![value]
                } else {
                    Vec::new()
                }
            }
        })
        .collect()
}
