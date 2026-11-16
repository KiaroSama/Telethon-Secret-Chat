//! Complete Desktop message JSON: headers, services, media, buttons and reactions.
use super::{
    error::Result,
    format::{DateContext, format_phone_number},
    json_actions::{push_service_action, serialize_todo_items},
    json_rich::{base64_url, boolean, serialize_rich_message},
    json_serialize::*,
    json_values::MessageValues,
    model::*,
    model_media::*,
    model_message::*,
    model_peers::Peers,
};
fn location(context: &JsonContext, point: &GeoPoint) -> Vec<u8> {
    serialize_object(
        context,
        &[
            ("latitude", number(point.latitude)),
            ("longitude", number(point.longitude)),
        ],
    )
}
fn push_document(v: &mut MessageValues<'_>, data: &Document) -> Result<()> {
    v.push_path(&data.file, "file")?;
    v.string("file_name", &data.name);
    v.number("file_size", data.file.size);
    if data.thumb.width > 0 {
        v.push_path(&data.thumb.file, "thumbnail")?;
        v.number("thumbnail_file_size", data.thumb.file.size);
    }
    if data.is_sticker {
        v.string("media_type", "sticker");
        v.string("sticker_emoji", &data.sticker_emoji);
    } else if data.is_video_message {
        v.string("media_type", "video_message");
    } else if data.is_voice_message {
        v.string("media_type", "voice_message");
    } else if data.is_animated {
        v.string("media_type", "animation");
    } else if data.is_video_file {
        v.string("media_type", "video_file");
    } else if data.is_audio_file {
        v.string("media_type", "audio_file");
        v.string("performer", &data.song_performer);
        v.string("title", &data.song_title);
    }
    v.string("mime_type", &data.mime);
    if data.duration != 0 {
        v.number("duration_seconds", data.duration);
    }
    if data.width != 0 && data.height != 0 {
        v.number("width", data.width);
        v.number("height", data.height);
    }
    v.push_spoiler(data.spoilered);
    v.push_ttl("self_destruct_period_seconds");
    Ok(())
}
fn poll(context: &mut JsonContext, data: &Poll) -> Vec<u8> {
    context.nesting.push(K_OBJECT);
    let mut answers = vec![];
    for answer in &data.answers {
        context.nesting.push(K_ARRAY);
        let text = serialize_text(context, &answer.text, false);
        answers.push(serialize_object(
            context,
            &[
                ("text", text),
                ("voters", number(answer.votes)),
                ("chosen", boolean(answer.my)),
            ],
        ));
        context.nesting.pop();
    }
    let serialized = serialize_array(context, &answers);
    context.nesting.pop();
    let question = serialize_text(context, &data.question, false);
    serialize_object(
        context,
        &[
            ("question", question),
            ("closed", boolean(data.closed)),
            ("total_voters", number(data.total_votes)),
            ("answers", serialized),
        ],
    )
}
fn todo_list(context: &mut JsonContext, data: &TodoList) -> Vec<u8> {
    context.nesting.push(K_OBJECT);
    let serialized = serialize_todo_items(context, &data.items);
    context.nesting.pop();
    let title = serialize_text(context, &data.title, false);
    serialize_object(
        context,
        &[
            ("title", title),
            ("others_can_append", boolean(data.others_can_append)),
            ("others_can_complete", boolean(data.others_can_complete)),
            ("answers", serialized),
        ],
    )
}
fn nested_array(context: &mut JsonContext, values: &[Vec<u8>]) -> Vec<u8> {
    context.nesting.push(K_ARRAY);
    let result = serialize_array(context, values);
    context.nesting.pop();
    result
}
fn giveaway_start(
    context: &mut JsonContext,
    data: &GiveawayStart,
    dates: &DateContext,
) -> Result<Vec<u8>> {
    let channels = nested_array(
        context,
        &data.channels.iter().map(|c| number(*c)).collect::<Vec<_>>(),
    );
    let countries = nested_array(
        context,
        &data
            .countries
            .iter()
            .map(serialize_string)
            .collect::<Vec<_>>(),
    );
    Ok(serialize_object(
        context,
        &[
            ("quantity", number(data.quantity)),
            ("months", number(data.months)),
            ("until_date", serialize_date(data.until_date, dates)?),
            ("channels", channels),
            ("countries", countries),
            ("additional_prize", serialize_string(&data.additional_prize)),
            ("stars", number(data.credits)),
            ("is_only_new_subscribers", boolean(!data.all)),
        ],
    ))
}
fn giveaway_results(
    context: &mut JsonContext,
    data: &GiveawayResults,
    dates: &DateContext,
) -> Result<Vec<u8>> {
    let winners = nested_array(
        context,
        &data.winners.iter().map(|c| number(*c)).collect::<Vec<_>>(),
    );
    Ok(serialize_object(
        context,
        &[
            ("channel", number(data.channel)),
            ("winners", winners),
            ("additional_prize", serialize_string(&data.additional_prize)),
            ("until_date", serialize_date(data.until_date, dates)?),
            ("launch_message_id", number(data.launch_id)),
            (
                "additional_peers_count",
                number(data.additional_peers_count),
            ),
            ("winners_count", number(data.winners_count)),
            ("unclaimed_count", number(data.unclaimed_count)),
            ("months", number(data.months)),
            ("stars", number(data.credits)),
            ("is_refunded", boolean(data.refunded)),
            ("is_only_new_subscribers", boolean(!data.all)),
        ],
    ))
}
fn push_media(
    v: &mut MessageValues<'_>,
    content: Option<&MediaContent>,
    domain: &str,
    dates: &DateContext,
) -> Result<()> {
    let Some(content) = content else {
        return Ok(());
    };
    use MediaContent as C;
    match content {
        C::Photo(data) => {
            v.push_photo(&data.image)?;
            v.push_spoiler(data.spoilered);
            v.push_ttl("self_destruct_period_seconds");
        }
        C::Document(data) => push_document(v, data)?,
        C::SharedContact(data) => {
            v.bare(
                "contact_information",
                serialize_object(
                    v.context,
                    &[
                        ("first_name", serialize_string(&data.info.first_name)),
                        ("last_name", serialize_string(&data.info.last_name)),
                        (
                            "phone_number",
                            serialize_string(format_phone_number(
                                &data.info.phone_number,
                                &v.context.countries,
                            )),
                        ),
                    ],
                ),
            );
            if !data.vcard.content.is_empty() {
                v.push_path(&data.vcard, "contact_vcard")?;
                v.number("contact_vcard_file_size", data.vcard.size);
            }
        }
        C::GeoPoint(data) => {
            v.bare(
                "location_information",
                if data.valid {
                    location(v.context, data)
                } else {
                    b"null".to_vec()
                },
            );
            v.push_ttl("live_location_period_seconds");
        }
        C::Venue(data) => {
            v.string("place_name", &data.title);
            v.string("address", &data.address);
            if data.point.valid {
                v.bare("location_information", location(v.context, &data.point));
            }
        }
        C::Game(data) => {
            v.string("game_title", &data.title);
            v.string("game_description", &data.description);
            if data.bot_id != 0 && !data.short_name.is_empty() {
                let bot = v.user_of(data.bot_id);
                if bot.is_bot && !bot.username.is_empty() {
                    v.string(
                        "game_link",
                        format!("{domain}{}?game={}", bot.username, data.short_name),
                    );
                }
            }
        }
        C::Invoice(data) => {
            let invoice = serialize_object(
                v.context,
                &[
                    ("title", serialize_string(&data.title)),
                    ("description", serialize_string(&data.description)),
                    ("amount", number(data.amount)),
                    ("currency", serialize_string(&data.currency)),
                    (
                        "receipt_message_id",
                        if data.receipt_msg_id != 0 {
                            number(data.receipt_msg_id)
                        } else {
                            vec![]
                        },
                    ),
                ],
            );
            v.string("invoice_information", invoice);
        }
        C::Poll(data) => {
            let value = poll(v.context, data);
            v.bare("poll", value);
        }
        C::TodoList(data) => {
            let value = todo_list(v.context, data);
            v.bare("todo_list", value);
        }
        C::GiveawayStart(data) => {
            let value = giveaway_start(v.context, data, dates)?;
            v.bare("giveaway_information", value);
        }
        C::GiveawayResults(data) => {
            let value = giveaway_results(v.context, data, dates)?;
            v.bare("giveaway_results", value);
        }
        C::PaidMedia(data) => v.number("paid_stars_amount", data.stars),
        C::UnsupportedMedia(_) => {}
    }
    Ok(())
}
fn button(context: &JsonContext, entry: &HistoryMessageMarkupButton) -> Vec<u8> {
    let mut pairs = vec![("type", serialize_string(entry.type_to_string()))];
    if !entry.text.is_empty() {
        pairs.push(("text", serialize_string(&entry.text)));
    }
    if !entry.data.is_empty() {
        if matches!(
            entry.r#type,
            MarkupButtonType::Callback | MarkupButtonType::CallbackWithPassword
        ) {
            pairs.push(("dataBase64", serialize_string(base64_url(&entry.data))));
            pairs.push(("data", serialize_string("")));
        } else {
            pairs.push(("data", serialize_string(&entry.data)));
        }
    }
    if !entry.forward_text.is_empty() {
        pairs.push(("forward_text", serialize_string(&entry.forward_text)));
    }
    if entry.button_id != 0 {
        pairs.push(("button_id", number(entry.button_id)));
    }
    serialize_object(context, &pairs)
}
fn inline_buttons(context: &mut JsonContext, rows: &[Vec<HistoryMessageMarkupButton>]) -> Vec<u8> {
    context.nesting.push(K_ARRAY);
    let mut serialized = vec![];
    for row in rows {
        context.nesting.push(K_ARRAY);
        let buttons = row.iter().map(|b| button(context, b)).collect::<Vec<_>>();
        context.nesting.pop();
        serialized.push(serialize_array(context, &buttons));
    }
    context.nesting.pop();
    serialize_array(context, &serialized)
}
fn reaction(
    v: &mut MessageValues<'_>,
    reaction: &Reaction,
    dates: &DateContext,
) -> Result<Vec<u8>> {
    v.context.nesting.push(K_OBJECT);
    let result = (|| {
        let mut pairs = vec![
            ("type", serialize_string(reaction.type_to_string())),
            ("count", number(reaction.count)),
        ];
        if reaction.r#type == ReactionType::Emoji {
            pairs.push(("emoji", serialize_string(&reaction.emoji)));
        } else if reaction.r#type == ReactionType::CustomEmoji {
            pairs.push(("document_id", serialize_string(&reaction.document_id)));
        }
        if !reaction.recent.is_empty() {
            v.context.nesting.push(K_ARRAY);
            let mut recents = vec![];
            for recent in &reaction.recent {
                let from = v.wrap_peer_name(recent.peer_id);
                let date = serialize_date(recent.date, dates)?;
                v.context.nesting.push(K_ARRAY);
                recents.push(serialize_object(
                    v.context,
                    &[
                        ("from", from),
                        ("from_id", MessageValues::wrap_peer_id(recent.peer_id)),
                        ("date", date),
                    ],
                ));
                v.context.nesting.pop();
            }
            pairs.push(("recent", serialize_array(v.context, &recents)));
            v.context.nesting.pop();
        }
        Ok(serialize_object(v.context, &pairs))
    })();
    v.context.nesting.pop();
    result
}
pub fn serialize_message(
    context: &mut JsonContext,
    message: &Message,
    peers: &Peers,
    domain: &str,
    dates: &DateContext,
) -> Result<Vec<u8>> {
    if matches!(
        message.media.content,
        Some(MediaContent::UnsupportedMedia(_))
    ) && message.rich_message.is_none()
    {
        return Ok(serialize_object(
            context,
            &[
                ("id", number(message.id)),
                ("type", serialize_string("unsupported")),
            ],
        ));
    }
    let date = serialize_date(message.date, dates)?;
    let mut v = MessageValues::new(context, message, peers);
    v.values = vec![
        ("id".into(), number(message.id)),
        (
            "type".into(),
            serialize_string(if message.action.content.is_some() {
                "service"
            } else {
                "message"
            }),
        ),
        ("date".into(), date),
        ("date_unixtime".into(), serialize_date_raw(message.date)),
    ];
    v.context.nesting.push(K_OBJECT);
    let result = fill_message(&mut v, message, domain, dates);
    v.context.nesting.pop();
    result?;
    Ok(serialize_object(v.context, &v.values))
}
fn fill_message(
    v: &mut MessageValues<'_>,
    message: &Message,
    domain: &str,
    dates: &DateContext,
) -> Result<()> {
    if message.edited != 0 {
        v.bare("edited", serialize_date(message.edited, dates)?);
        v.bare("edited_unixtime", serialize_date_raw(message.edited));
    }
    push_service_action(v, message.action.content.as_ref())?;
    if message.action.content.is_none() {
        v.push_from("from");
        v.string("author", &message.signature);
        if message.forwarded_from_id != 0 {
            v.bare(
                "forwarded_from",
                v.wrap_peer_name(message.forwarded_from_id),
            );
            v.peer("forwarded_from_id", message.forwarded_from_id);
        } else if !message.forwarded_from_name.is_empty() {
            v.bare(
                "forwarded_from",
                string_allow_null(&message.forwarded_from_name),
            );
        }
        if message.saved_from_chat_id != 0 {
            v.bare("saved_from", v.wrap_peer_name(message.saved_from_chat_id));
        }
        v.push_reply_to_msg_id("reply_to_message_id");
        if message.via_bot_id != 0 {
            v.string(
                "via_bot",
                format_username(&v.user_of(message.via_bot_id).username),
            );
        }
    }
    push_media(v, message.media.content.as_ref(), domain, dates)?;
    if let Some(rich) = &message.rich_message {
        let rich = serialize_rich_message(v.context, rich, dates)?;
        v.bare("rich_message", rich);
    } else {
        let text = serialize_text(v.context, &message.text, false);
        v.bare("text", text);
        let entities = serialize_text(v.context, &message.text, true);
        v.bare("text_entities", entities);
    }
    if !message.inline_button_rows.is_empty() {
        let buttons = inline_buttons(v.context, &message.inline_button_rows);
        v.bare("inline_bot_buttons", buttons);
    }
    if !message.reactions.is_empty() {
        v.context.nesting.push(K_ARRAY);
        let reactions = message
            .reactions
            .iter()
            .map(|r| reaction(v, r, dates))
            .collect::<Result<Vec<_>>>();
        let reactions = reactions.map(|r| serialize_array(v.context, &r));
        v.context.nesting.pop();
        v.bare("reactions", reactions?);
    }
    Ok(())
}
