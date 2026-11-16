//! Native TL rich text and inline button parsing.
use super::{
    model::{array, boolean, integer, kind, string},
    model_document::bytes,
    model_peers::type_is,
    model_rich::*,
};
use serde_json::Value;
fn enum_value<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    serde_json::from_value(Value::String(name.into())).ok()
}
pub fn parse_inline_button_action(data: &Value) -> Option<InlineButtonAction> {
    let mut action = InlineButtonAction::default();
    let name = kind(data)
        .trim_start_matches("InlineButtonType")
        .trim_start_matches("inlineButtonType");
    match name {
        "Url" => {
            action.r#type = InlineButtonActionType::Url;
            action.url = string(data, "url").into();
        }
        "UrlAuth" => {
            action.r#type = InlineButtonActionType::Auth;
            action.url = string(data, "url").into();
            action.button_id = integer(data, "button_id");
            action.forward_text = data["fwd_text"].as_str().map(str::to_owned);
        }
        "WebView" => {
            action.r#type = InlineButtonActionType::WebView;
            action.url = string(data, "url").into();
        }
        "Callback" => {
            action.requires_password = boolean(data, "requires_password");
            action.r#type = if action.requires_password {
                InlineButtonActionType::CallbackWithPassword
            } else {
                InlineButtonActionType::Callback
            };
            action.callback_data = bytes(&data["data"]);
        }
        "Game" => action.r#type = InlineButtonActionType::Game,
        "Buy" => action.r#type = InlineButtonActionType::Buy,
        "SwitchInline" => {
            action.same_peer = boolean(data, "same_peer");
            action.r#type = if action.same_peer {
                InlineButtonActionType::SwitchInlineSame
            } else {
                InlineButtonActionType::SwitchInline
            };
            action.query = string(data, "query").into();
            if data.get("peer_types").is_some_and(|v| !v.is_null()) {
                action.peer_types = Some(
                    array(data, "peer_types")
                        .iter()
                        .filter_map(|v| {
                            enum_value(
                                kind(v)
                                    .trim_start_matches("InlineQueryPeerType")
                                    .trim_start_matches("inlineQueryPeerType"),
                            )
                        })
                        .collect(),
                );
            }
        }
        "UserProfile" => {
            action.r#type = InlineButtonActionType::UserProfile;
            action.user_id = integer(data, "user_id");
        }
        "Copy" => {
            action.r#type = InlineButtonActionType::CopyText;
            action.copy_text = string(data, "copy_text").into();
        }
        "Disabled" => action.r#type = InlineButtonActionType::Disabled,
        _ => return None,
    }
    Some(action)
}
pub fn parse_rich_button(
    text: &Value,
    button_type: &Value,
    style: &Value,
    inline_button: bool,
) -> Option<RichText> {
    let action = parse_inline_button_action(button_type)?;
    let link_allowed = matches!(
        action.r#type,
        InlineButtonActionType::Callback
            | InlineButtonActionType::CallbackWithPassword
            | InlineButtonActionType::Disabled
    );
    let style = if style.is_null() {
        None
    } else {
        Some(if inline_button && boolean(style, "link") && link_allowed {
            RichButtonStyle::Link
        } else if boolean(style, "bg_danger") {
            RichButtonStyle::Danger
        } else if boolean(style, "bg_primary") {
            RichButtonStyle::Primary
        } else if boolean(style, "bg_success") {
            RichButtonStyle::Success
        } else {
            RichButtonStyle::Default
        })
    };
    Some(RichText {
        r#type: RichTextType::Button,
        children: vec![parse_rich_text(text)],
        button: Some(RichButtonPayload { action, style }),
        ..Default::default()
    })
}
fn wrapper(kind: RichTextType, child: &Value) -> RichText {
    RichText {
        r#type: kind,
        children: vec![parse_rich_text(child)],
        ..Default::default()
    }
}
pub fn parse_rich_text(text: &Value) -> RichText {
    let name = kind(text)
        .trim_start_matches("Text")
        .trim_start_matches("text");
    match name {
        "Bold" | "Italic" | "Underline" | "Strike" | "Fixed" | "Subscript" | "Superscript"
        | "Marked" | "Spoiler" | "Mention" | "Hashtag" | "BotCommand" | "Cashtag" | "AutoUrl"
        | "AutoEmail" | "AutoPhone" | "BankCard" | "TonAddress" => {
            wrapper(enum_value(name).unwrap_or_default(), &text["text"])
        }
        "Plain" => RichText {
            r#type: RichTextType::Plain,
            text: string(text, "text").into(),
            ..Default::default()
        },
        "Url" => RichText {
            data: string(text, "url").into(),
            id: integer(text, "webpage_id"),
            ..wrapper(RichTextType::Url, &text["text"])
        },
        "Email" => RichText {
            data: string(text, "email").into(),
            ..wrapper(RichTextType::Email, &text["text"])
        },
        "Concat" => RichText {
            r#type: RichTextType::Concat,
            children: array(text, "texts").iter().map(parse_rich_text).collect(),
            ..Default::default()
        },
        "Phone" => RichText {
            data: string(text, "phone").into(),
            ..wrapper(RichTextType::Phone, &text["text"])
        },
        "Image" => RichText {
            r#type: RichTextType::InlineImage,
            id: integer(text, "document_id"),
            width: integer(text, "w"),
            height: integer(text, "h"),
            ..Default::default()
        },
        "Anchor" => RichText {
            data: string(text, "name").into(),
            ..wrapper(RichTextType::Anchor, &text["text"])
        },
        "Math" => RichText {
            r#type: RichTextType::Math,
            data: string(text, "source").into(),
            ..Default::default()
        },
        "CustomEmoji" => RichText {
            r#type: RichTextType::CustomEmoji,
            text: string(text, "alt").into(),
            id: integer(text, "document_id"),
            custom_emoji_data: integer(text, "document_id").to_string(),
            ..Default::default()
        },
        "MentionName" => RichText {
            id: integer(text, "user_id"),
            ..wrapper(RichTextType::MentionName, &text["text"])
        },
        "Date" => RichText {
            date: integer(text, "date"),
            relative: boolean(text, "relative"),
            short_time: boolean(text, "short_time"),
            long_time: boolean(text, "long_time"),
            short_date: boolean(text, "short_date"),
            long_date: boolean(text, "long_date"),
            day_of_week: boolean(text, "day_of_week"),
            ..wrapper(RichTextType::FormattedDate, &text["text"])
        },
        "Diff" => RichText {
            r#type: RichTextType::Diff,
            unsupported: true,
            children: vec![parse_rich_text(&text["text"])],
            old_children: vec![parse_rich_text(&text["old_text"])],
            ..Default::default()
        },
        "Button" => parse_rich_button(&text["text"], &text["type"], &text["style"], true)
            .unwrap_or_else(|| parse_rich_text(&text["text"])),
        _ => RichText::default(),
    }
}
pub fn parse_rich_caption(caption: &Value) -> RichCaption {
    RichCaption {
        text: parse_rich_text(&caption["text"]),
        credit: parse_rich_text(&caption["credit"]),
    }
}
pub fn parse_rich_channel(chat: &Value) -> RichChannel {
    let mut result = RichChannel {
        id: integer(chat, "id"),
        ..Default::default()
    };
    if type_is(chat, "ChatEmpty") {
        return result;
    }
    result.title = chat["title"].as_str().map(str::to_owned);
    result.source = enum_value(kind(chat)).unwrap_or_default();
    if type_is(chat, "Channel") || type_is(chat, "ChannelForbidden") {
        result.broadcast = boolean(chat, "broadcast");
        result.megagroup = boolean(chat, "megagroup");
        result.monoforum = boolean(chat, "monoforum");
        result.access_hash = chat["access_hash"].as_i64();
        if type_is(chat, "Channel") {
            result.username = chat["username"].as_str().map(str::to_owned);
        }
    } else if type_is(chat, "Community") || type_is(chat, "CommunityForbidden") {
        result.access_hash = chat["access_hash"].as_i64();
    }
    result
}
pub fn parse_rich_map_point(point: &Value) -> RichMapPoint {
    let mut result = RichMapPoint::default();
    if type_is(point, "GeoPoint") || type_is(point, "InputGeoPoint") {
        result.source = if type_is(point, "GeoPoint") {
            RichMapPointSource::GeoPoint
        } else {
            RichMapPointSource::InputGeoPoint
        };
        result.latitude = point["lat"].as_f64().unwrap_or(0.0);
        result.longitude = point["long"].as_f64().unwrap_or(0.0);
        result.accuracy_radius = point["accuracy_radius"].as_i64();
        if type_is(point, "GeoPoint") {
            result.access_hash = point["access_hash"].as_i64();
        }
    } else if type_is(point, "InputGeoPointEmpty") {
        result.source = RichMapPointSource::InputGeoPointEmpty;
    }
    result
}
