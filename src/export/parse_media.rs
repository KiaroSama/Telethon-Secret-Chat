//! TL-JSON media parsing, preserving Desktop's null, TTL, and preview rules.
use super::{
    error::Result,
    format::DateContext,
    model::*,
    model_document::{bytes, parse_document, parse_photo},
    model_media::*,
    model_peers::type_is,
};
use serde_json::Value;
pub fn parse_text_with_entities(v: &Value) -> Vec<TextPart> {
    if v.is_null() {
        vec![]
    } else {
        parse_text(string(v, "text"), array(v, "entities"))
    }
}
pub fn parse_shared_contact(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
) -> SharedContact {
    let mut result = SharedContact::default();
    result.info.user_id = integer(data, "user_id");
    result.info.first_name = string(data, "first_name").into();
    result.info.last_name = string(data, "last_name").into();
    result.info.phone_number = string(data, "phone_number").into();
    let vcard = string(data, "vcard");
    if !vcard.is_empty() {
        result.vcard.content = vcard.as_bytes().to_vec();
        result.vcard.size = result.vcard.content.len() as u64;
        context.contacts += 1;
        result.vcard.suggested_path =
            format!("{folder}contacts/contact_{}.vcard", context.contacts);
    }
    result
}
pub fn parse_geo_point(data: &Value) -> GeoPoint {
    if type_is(data, "GeoPoint") {
        GeoPoint {
            latitude: data["lat"].as_f64().unwrap_or(0.0),
            longitude: data["long"].as_f64().unwrap_or(0.0),
            valid: true,
        }
    } else {
        GeoPoint::default()
    }
}
pub fn parse_venue(data: &Value) -> Venue {
    Venue {
        point: parse_geo_point(&data["geo"]),
        title: string(data, "title").into(),
        address: string(data, "address").into(),
    }
}
pub fn parse_game(data: &Value, bot_id: i64) -> Game {
    Game {
        id: integer(data, "id"),
        short_name: string(data, "short_name").into(),
        title: string(data, "title").into(),
        description: string(data, "description").into(),
        bot_id,
    }
}
pub fn parse_invoice(data: &Value) -> Invoice {
    Invoice {
        title: string(data, "title").into(),
        description: string(data, "description").into(),
        currency: string(data, "currency").into(),
        amount: integer(data, "total_amount"),
        receipt_msg_id: integer(data, "receipt_msg_id"),
    }
}
pub fn parse_paid_media(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    date: i64,
    dates: &DateContext,
) -> Result<PaidMedia> {
    let mut result = PaidMedia {
        stars: integer(data, "stars_amount"),
        extended: vec![],
    };
    for extended in array(data, "extended_media") {
        result
            .extended
            .push(if type_is(extended, "MessageExtendedMedia") {
                Some(parse_media(
                    context,
                    &extended["media"],
                    folder,
                    date,
                    dates,
                )?)
            } else {
                None
            });
    }
    Ok(result)
}
pub fn parse_poll(data: &Value) -> Poll {
    let mut result = Poll::default();
    let poll = &data["poll"];
    if type_is(poll, "Poll") {
        result.id = integer(poll, "id");
        result.question = parse_text_with_entities(&poll["question"]);
        result.closed = boolean(poll, "closed");
        for answer in array(poll, "answers") {
            result.answers.push(if type_is(answer, "PollAnswer") {
                PollAnswer {
                    text: parse_text_with_entities(&answer["text"]),
                    option: bytes(&answer["option"]),
                    ..Default::default()
                }
            } else {
                PollAnswer::default()
            });
        }
    }
    let results = &data["results"];
    if type_is(results, "PollResults") {
        result.total_votes = integer(results, "total_voters");
        for voters in array(results, "results") {
            if let Some(answer) = result
                .answers
                .iter_mut()
                .find(|a| a.option == bytes(&voters["option"]))
            {
                answer.votes = integer(voters, "voters");
                answer.my |= boolean(voters, "chosen");
            }
        }
    }
    result
}
pub fn parse_todo_list_item(item: &Value) -> TodoListItem {
    TodoListItem {
        text: parse_text_with_entities(&item["title"]),
        id: integer(item, "id"),
    }
}
pub fn parse_todo_list(data: &Value) -> TodoList {
    let todo = &data["todo"];
    if type_is(todo, "TodoList") {
        TodoList {
            others_can_append: boolean(todo, "others_can_append"),
            others_can_complete: boolean(todo, "others_can_complete"),
            title: parse_text_with_entities(&todo["title"]),
            items: array(todo, "list")
                .iter()
                .map(parse_todo_list_item)
                .collect(),
        }
    } else {
        TodoList::default()
    }
}
pub fn parse_giveaway_start(data: &Value) -> GiveawayStart {
    GiveawayStart {
        until_date: integer(data, "until_date"),
        credits: integer(data, "stars"),
        quantity: integer(data, "quantity"),
        months: integer(data, "months"),
        all: !boolean(data, "only_new_subscribers"),
        channels: array(data, "channels")
            .iter()
            .filter_map(Value::as_i64)
            .collect(),
        countries: array(data, "countries_iso2")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        additional_prize: string(data, "prize_description").into(),
    }
}
pub fn parse_giveaway_results(data: &Value) -> GiveawayResults {
    GiveawayResults {
        channel: integer(data, "channel_id"),
        until_date: integer(data, "until_date"),
        launch_id: integer(data, "launch_msg_id"),
        additional_peers_count: integer(data, "additional_peers_count"),
        winners_count: integer(data, "winners_count"),
        unclaimed_count: integer(data, "unclaimed_count"),
        months: integer(data, "months"),
        credits: integer(data, "stars"),
        refunded: boolean(data, "refunded"),
        all: !boolean(data, "only_new_subscribers"),
        winners: array(data, "winners")
            .iter()
            .filter_map(Value::as_i64)
            .collect(),
        additional_prize: string(data, "prize_description").into(),
    }
}
pub fn parse_media(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    date: i64,
    dates: &DateContext,
) -> Result<Media> {
    let mut result = Media::default();
    result.content = if type_is(data, "MessageMediaPhoto") {
        let mut photo = Photo::default();
        if data.get("photo").is_some_and(|v| !v.is_null()) {
            context.photos += 1;
            photo = parse_photo(
                &data["photo"],
                &format!(
                    "{folder}photos/{}",
                    prepare_photo_file_name(context.photos, date, dates)?
                ),
            );
        }
        photo.spoilered = boolean(data, "spoiler");
        if data.get("ttl_seconds").is_some_and(|v| !v.is_null()) {
            result.ttl = integer(data, "ttl_seconds");
            photo.image.file = File::default();
        }
        Some(MediaContent::Photo(photo))
    } else if type_is(data, "MessageMediaGeo") || type_is(data, "MessageMediaGeoLive") {
        if type_is(data, "MessageMediaGeoLive") {
            result.ttl = integer(data, "period");
        }
        Some(MediaContent::GeoPoint(parse_geo_point(&data["geo"])))
    } else if type_is(data, "MessageMediaContact") {
        Some(MediaContent::SharedContact(parse_shared_contact(
            context, data, folder,
        )))
    } else if type_is(data, "MessageMediaUnsupported") {
        Some(MediaContent::UnsupportedMedia(UnsupportedMedia {}))
    } else if type_is(data, "MessageMediaDocument") {
        let mut document = if data.get("document").is_some_and(|v| !v.is_null()) {
            parse_document(context, &data["document"], folder, date, dates)?
        } else {
            Document::default()
        };
        if data.get("ttl_seconds").is_some_and(|v| !v.is_null()) {
            result.ttl = integer(data, "ttl_seconds");
            document.file = File::default();
        }
        document.spoilered = boolean(data, "spoiler");
        Some(MediaContent::Document(document))
    } else if type_is(data, "MessageMediaVenue") {
        Some(MediaContent::Venue(parse_venue(data)))
    } else if type_is(data, "MessageMediaGame") {
        Some(MediaContent::Game(parse_game(
            &data["game"],
            context.bot_id,
        )))
    } else if type_is(data, "MessageMediaInvoice") {
        Some(MediaContent::Invoice(parse_invoice(data)))
    } else if type_is(data, "MessageMediaPoll") {
        Some(MediaContent::Poll(parse_poll(data)))
    } else if type_is(data, "MessageMediaToDo") {
        Some(MediaContent::TodoList(parse_todo_list(data)))
    } else if type_is(data, "MessageMediaGiveaway") {
        Some(MediaContent::GiveawayStart(parse_giveaway_start(data)))
    } else if type_is(data, "MessageMediaGiveawayResults") {
        Some(MediaContent::GiveawayResults(parse_giveaway_results(data)))
    } else if type_is(data, "MessageMediaPaidMedia") {
        Some(MediaContent::PaidMedia(parse_paid_media(
            context, data, folder, date, dates,
        )?))
    } else {
        None
    };
    Ok(result)
}
