//! Byte-exact Desktop rich text and structural value serialization.
use super::{
    error::{ExportError, Result},
    format::DateContext,
    json_rich_metadata::*,
    json_serialize::*,
    model_rich::*,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
pub struct RichSerializeContext<'a> {
    pub json: &'a mut JsonContext,
    pub message: &'a RichMessage,
    pub dates: &'a DateContext,
}
pub fn base64_url(bytes: &[u8]) -> Vec<u8> {
    URL_SAFE_NO_PAD.encode(bytes).into_bytes()
}
pub fn id(value: i64) -> Vec<u8> {
    serialize_string(number(value as u64))
}
pub fn enum_name(value: impl Serialize) -> String {
    let raw = serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let chars = raw.chars().collect::<Vec<_>>();
    let mut result = String::new();
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase()
                || chars.get(i + 1).is_some_and(char::is_ascii_lowercase)
                    && chars[i - 1].is_ascii_uppercase())
        {
            result.push('_');
        }
        result.push(c.to_ascii_lowercase());
    }
    result.trim_end_matches('_').into()
}
pub fn boolean(value: bool) -> Vec<u8> {
    if value {
        b"true".to_vec()
    } else {
        b"false".to_vec()
    }
}
pub fn push(values: &mut Pairs, key: &str, value: Vec<u8>) {
    values.push((key.into(), value));
}
pub fn rich_array<T>(
    context: &mut RichSerializeContext<'_>,
    data: &[T],
    mut serializer: impl FnMut(&mut RichSerializeContext<'_>, &T) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(b"[]".to_vec());
    }
    context.json.nesting.push(K_ARRAY);
    let values = data
        .iter()
        .map(|value| serializer(context, value))
        .collect::<Result<Vec<_>>>();
    context.json.nesting.pop();
    Ok(serialize_array(context.json, &values?))
}
pub fn object(
    context: &mut RichSerializeContext<'_>,
    mut values: Pairs,
    fill: impl FnOnce(&mut RichSerializeContext<'_>, &mut Pairs) -> Result<()>,
) -> Result<Vec<u8>> {
    context.json.nesting.push(K_OBJECT);
    let result = fill(context, &mut values);
    context.json.nesting.pop();
    result?;
    Ok(serialize_object(context.json, &values))
}
fn child(context: &mut RichSerializeContext<'_>, children: &[RichText]) -> Result<Vec<u8>> {
    if children.len() != 1 {
        return Err(ExportError::InvalidRecord);
    }
    serialize_rich_text(context, &children[0])
}
fn text_type(kind: RichTextType) -> String {
    match kind {
        RichTextType::Strike => "strikethrough".into(),
        RichTextType::Fixed => "code".into(),
        RichTextType::Url => "text_link".into(),
        RichTextType::AutoUrl => "link".into(),
        RichTextType::AutoEmail => "email".into(),
        RichTextType::AutoPhone => "phone".into(),
        _ => enum_name(kind),
    }
}
fn button_action(
    context: &mut RichSerializeContext<'_>,
    action: &InlineButtonAction,
) -> Result<Vec<u8>> {
    object(
        context,
        vec![("type".into(), serialize_string(enum_name(action.r#type)))],
        |context, values| {
            use InlineButtonActionType as A;
            match action.r#type {
                A::Url | A::WebView => push(values, "url", serialize_string(&action.url)),
                A::Auth => {
                    push(values, "url", serialize_string(&action.url));
                    if let Some(text) = &action.forward_text {
                        push(values, "forward_text", serialize_string(text));
                    }
                    push(values, "button_id", number(action.button_id));
                }
                A::Callback | A::CallbackWithPassword => {
                    push(
                        values,
                        "requires_password",
                        boolean(action.requires_password),
                    );
                    push(
                        values,
                        "dataBase64",
                        serialize_string(base64_url(&action.callback_data)),
                    );
                    push(values, "data", serialize_string(""));
                }
                A::SwitchInline | A::SwitchInlineSame => {
                    push(values, "query", serialize_string(&action.query));
                    push(values, "same_peer", boolean(action.same_peer));
                    if let Some(peers) = &action.peer_types {
                        push(
                            values,
                            "peer_types",
                            rich_array(context, peers, |_, peer| {
                                Ok(serialize_string(enum_name(peer)))
                            })?,
                        );
                    }
                }
                A::UserProfile => push(values, "user_id", id(action.user_id)),
                A::CopyText => push(values, "copy_text", serialize_string(&action.copy_text)),
                _ => {}
            }
            Ok(())
        },
    )
}
fn append_button_fields(
    values: &mut Pairs,
    context: &mut RichSerializeContext<'_>,
    button: &RichText,
) -> Result<()> {
    let Some(payload) = &button.button else {
        return Err(ExportError::InvalidRecord);
    };
    push(values, "text", child(context, &button.children)?);
    push(values, "button", button_action(context, &payload.action)?);
    if let Some(style) = payload.style {
        push(values, "style", serialize_string(enum_name(style)));
    }
    Ok(())
}
pub fn serialize_rich_text(
    context: &mut RichSerializeContext<'_>,
    data: &RichText,
) -> Result<Vec<u8>> {
    object(
        context,
        vec![("type".into(), serialize_string(text_type(data.r#type)))],
        |context, values| {
            use RichTextType as T;
            match data.r#type {
                T::Empty => {}
                T::Plain => push(values, "text", serialize_string(&data.text)),
                T::Concat => push(
                    values,
                    "text",
                    serialize_rich_texts(context, &data.children)?,
                ),
                T::Bold
                | T::Italic
                | T::Underline
                | T::Strike
                | T::Fixed
                | T::Subscript
                | T::Superscript
                | T::Marked
                | T::Spoiler
                | T::Mention
                | T::Hashtag
                | T::BotCommand
                | T::Cashtag
                | T::AutoUrl
                | T::AutoEmail
                | T::AutoPhone
                | T::BankCard
                | T::TonAddress => push(values, "text", child(context, &data.children)?),
                T::Url => {
                    push(values, "href", serialize_string(&data.data));
                    if data.id != 0 {
                        push(values, "webpage_id", id(data.id));
                    }
                    push(values, "text", child(context, &data.children)?);
                }
                T::Email | T::Phone | T::Anchor => {
                    push(
                        values,
                        match data.r#type {
                            T::Email => "email",
                            T::Phone => "phone",
                            _ => "name",
                        },
                        serialize_string(&data.data),
                    );
                    push(values, "text", child(context, &data.children)?);
                }
                T::Math => push(values, "source", serialize_string(&data.data)),
                T::CustomEmoji => {
                    push(values, "text", serialize_string(&data.text));
                    push(
                        values,
                        "document_id",
                        serialize_string(&data.custom_emoji_data),
                    );
                }
                T::MentionName => {
                    push(values, "user_id", id(data.id));
                    push(values, "text", child(context, &data.children)?);
                }
                T::FormattedDate => {
                    push(values, "text", child(context, &data.children)?);
                    push(values, "date", serialize_date(data.date, context.dates)?);
                    push(values, "date_unixtime", serialize_date_raw(data.date));
                    for (key, value) in [
                        ("relative", data.relative),
                        ("short_time", data.short_time),
                        ("long_time", data.long_time),
                        ("short_date", data.short_date),
                        ("long_date", data.long_date),
                        ("day_of_week", data.day_of_week),
                    ] {
                        push(values, key, boolean(value));
                    }
                }
                T::InlineImage => {
                    push(values, "document_id", id(data.id));
                    append_document_metadata(
                        values,
                        context.message.documents.get(&data.id),
                        false,
                    );
                    push(values, "width", number(data.width));
                    push(values, "height", number(data.height));
                }
                T::Diff => {
                    push(values, "unsupported", b"true".to_vec());
                    push(values, "text", child(context, &data.children)?);
                    push(values, "old_text", child(context, &data.old_children)?);
                }
                T::Button => append_button_fields(values, context, data)?,
            }
            Ok(())
        },
    )
}
pub fn serialize_rich_texts(
    context: &mut RichSerializeContext<'_>,
    data: &[RichText],
) -> Result<Vec<u8>> {
    rich_array(context, data, serialize_rich_text)
}
pub fn rich_button(context: &mut RichSerializeContext<'_>, data: &RichText) -> Result<Vec<u8>> {
    object(context, vec![], |context, values| {
        append_button_fields(values, context, data)
    })
}
pub fn caption(context: &mut RichSerializeContext<'_>, data: &RichCaption) -> Result<Vec<u8>> {
    object(context, vec![], |context, values| {
        push(values, "text", serialize_rich_text(context, &data.text)?);
        push(
            values,
            "credit",
            serialize_rich_text(context, &data.credit)?,
        );
        Ok(())
    })
}
pub fn list_item(context: &mut RichSerializeContext<'_>, data: &RichListItem) -> Result<Vec<u8>> {
    let mut values = vec![
        (
            "task_state".into(),
            serialize_string(enum_name(data.task_state)),
        ),
        ("content".into(), serialize_string(enum_name(data.content))),
    ];
    context.json.nesting.push(K_OBJECT);
    let result = if data.content == RichListItemContent::Text {
        serialize_rich_text(context, data.text.as_ref().unwrap_or(&RichText::default()))
            .map(|v| ("text", v))
    } else {
        super::json_rich_block::serialize_rich_blocks(context, &data.blocks).map(|v| ("blocks", v))
    };
    context.json.nesting.pop();
    let (key, value) = result?;
    push(&mut values, key, value);
    if let Some(num) = &data.num {
        push(&mut values, "num", serialize_string(num));
    }
    if let Some(value) = data.value {
        push(&mut values, "value", number(value));
    }
    if let Some(kind) = &data.r#type {
        push(&mut values, "item_type", serialize_string(kind));
    }
    Ok(serialize_object(context.json, &values))
}
pub fn table_cell(context: &mut RichSerializeContext<'_>, data: &RichTableCell) -> Result<Vec<u8>> {
    let mut values = vec![];
    if let Some(text) = &data.text {
        context.json.nesting.push(K_OBJECT);
        let text = serialize_rich_text(context, text);
        context.json.nesting.pop();
        push(&mut values, "text", text?);
    }
    if let Some(value) = data.colspan {
        push(&mut values, "colspan", number(value));
    }
    if let Some(value) = data.rowspan {
        push(&mut values, "rowspan", number(value));
    }
    push(&mut values, "header", boolean(data.header));
    push(
        &mut values,
        "align",
        serialize_string(enum_name(data.alignment)),
    );
    push(
        &mut values,
        "vertical_align",
        serialize_string(enum_name(data.vertical_alignment)),
    );
    Ok(serialize_object(context.json, &values))
}
pub fn table_row(context: &mut RichSerializeContext<'_>, data: &RichTableRow) -> Result<Vec<u8>> {
    context.json.nesting.push(K_OBJECT);
    let cells = rich_array(context, &data.cells, table_cell);
    context.json.nesting.pop();
    Ok(serialize_object(context.json, &[("cells", cells?)]))
}
pub fn related_article(
    context: &mut RichSerializeContext<'_>,
    data: &RichRelatedArticle,
) -> Result<Vec<u8>> {
    let mut values = vec![
        ("url".into(), serialize_string(&data.url)),
        ("webpage_id".into(), id(data.webpage_id)),
    ];
    if let Some(text) = &data.title {
        push(&mut values, "title", serialize_string(text));
    }
    if let Some(text) = &data.description {
        push(&mut values, "description", serialize_string(text));
    }
    if let Some(photo) = data.photo_id {
        push(&mut values, "photo_id", id(photo));
        append_photo_metadata(
            &mut values,
            context.message.photos.get(&photo),
            [
                "photo",
                "photo_skip_reason",
                "photo_file_size",
                "photo_width",
                "photo_height",
            ],
        );
    }
    if let Some(text) = &data.author {
        push(&mut values, "author", serialize_string(text));
    }
    if let Some(date) = data.published_date {
        push(
            &mut values,
            "published_date",
            serialize_date(date, context.dates)?,
        );
        push(
            &mut values,
            "published_date_unixtime",
            serialize_date_raw(date),
        );
    }
    Ok(serialize_object(context.json, &values))
}
pub fn channel(context: &RichSerializeContext<'_>, data: &RichChannel) -> Vec<u8> {
    let mut values = vec![
        (
            "source_type".into(),
            serialize_string(enum_name(data.source)),
        ),
        ("id".into(), id(data.id)),
    ];
    if let Some(value) = data.access_hash {
        push(&mut values, "access_hash", serialize_string(number(value)));
    }
    if let Some(text) = &data.title {
        push(&mut values, "title", serialize_string(text));
    }
    if let Some(text) = &data.username {
        push(&mut values, "username", serialize_string(text));
    }
    if matches!(
        data.source,
        RichChannelSource::Channel | RichChannelSource::ChannelForbidden
    ) {
        push(&mut values, "broadcast", boolean(data.broadcast));
        push(&mut values, "megagroup", boolean(data.megagroup));
        push(&mut values, "monoforum", boolean(data.monoforum));
    }
    serialize_object(context.json, &values)
}
pub fn map_point(context: &RichSerializeContext<'_>, data: &RichMapPoint) -> Vec<u8> {
    let mut values = vec![(
        "source_type".into(),
        serialize_string(enum_name(data.source)),
    )];
    if matches!(
        data.source,
        RichMapPointSource::GeoPoint | RichMapPointSource::InputGeoPoint
    ) {
        push(&mut values, "latitude", float_allow_null(data.latitude));
        push(&mut values, "longitude", float_allow_null(data.longitude));
    }
    if let Some(value) = data.access_hash {
        push(&mut values, "access_hash", serialize_string(number(value)));
    }
    if let Some(value) = data.accuracy_radius {
        push(&mut values, "accuracy_radius", number(value));
    }
    serialize_object(context.json, &values)
}
pub fn serialize_rich_message(
    context: &mut JsonContext,
    data: &RichMessage,
    dates: &DateContext,
) -> Result<Vec<u8>> {
    let mut rich = RichSerializeContext {
        json: context,
        message: data,
        dates,
    };
    object(
        &mut rich,
        vec![
            ("rtl".into(), boolean(data.rtl)),
            ("part".into(), boolean(data.part)),
        ],
        |context, values| {
            push(
                values,
                "blocks",
                super::json_rich_block::serialize_rich_blocks(context, &data.blocks)?,
            );
            Ok(())
        },
    )
}
