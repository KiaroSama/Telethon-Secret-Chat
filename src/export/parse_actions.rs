//! TL service action parsing, including Desktop's deliberate export quirks.
use super::{
    error::Result,
    format::{DateContext, fill_amount_and_currency},
    model::*,
    model_actions::*,
    model_document::parse_photo,
    model_peers::type_is,
    parse_media::{parse_text_with_entities, parse_todo_list_item},
};
use serde_json::Value;
pub fn parse_star_gift(gift: &Value) -> ActionStarGift {
    let mut result = ActionStarGift {
        gift_id: integer(gift, "id"),
        ..Default::default()
    };
    if type_is(gift, "StarGift") {
        result.stars = integer(gift, "stars");
        result.limited = boolean(gift, "limited");
    }
    result
}
fn photo(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    date: i64,
    dates: &DateContext,
) -> Result<Photo> {
    context.photos += 1;
    Ok(parse_photo(
        &data["photo"],
        &format!(
            "{folder}photos/{}",
            prepare_photo_file_name(context.photos, date, dates)?
        ),
    ))
}
fn ids(data: &Value, key: &str) -> Vec<i64> {
    array(data, key).iter().filter_map(Value::as_i64).collect()
}
pub fn parse_service_action(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    date: i64,
    dates: &DateContext,
) -> Result<ServiceAction> {
    let name = kind(data)
        .trim_start_matches("messageAction")
        .trim_start_matches("MessageAction");
    let content = match name {
        "ChatCreate" => Action::ChatCreate(ActionChatCreate {
            title: string(data, "title").into(),
            user_ids: ids(data, "users"),
        }),
        "ChatEditTitle" => Action::ChatEditTitle(ActionChatEditTitle {
            title: string(data, "title").into(),
        }),
        "ChatEditPhoto" => Action::ChatEditPhoto(ActionChatEditPhoto {
            photo: photo(context, data, folder, date, dates)?,
        }),
        "ChatDeletePhoto" => Action::ChatDeletePhoto(ActionChatDeletePhoto {}),
        "ChatAddUser" => Action::ChatAddUser(ActionChatAddUser {
            user_ids: ids(data, "users"),
        }),
        "ChatDeleteUser" => Action::ChatDeleteUser(ActionChatDeleteUser {
            user_id: integer(data, "user_id"),
        }),
        "ChatJoinedByLink" => Action::ChatJoinedByLink(ActionChatJoinedByLink {
            inviter_id: integer(data, "inviter_id"),
        }),
        "ChannelCreate" => Action::ChannelCreate(ActionChannelCreate {
            title: string(data, "title").into(),
        }),
        "ChatMigrateTo" => Action::ChatMigrateTo(ActionChatMigrateTo {
            channel_id: integer(data, "channel_id"),
        }),
        "ChannelMigrateFrom" => Action::ChannelMigrateFrom(ActionChannelMigrateFrom {
            title: string(data, "title").into(),
            chat_id: integer(data, "chat_id"),
        }),
        "PinMessage" => Action::PinMessage(ActionPinMessage {}),
        "HistoryClear" => Action::HistoryClear(ActionHistoryClear {}),
        "GameScore" => Action::GameScore(ActionGameScore {
            game_id: integer(data, "game_id"),
            score: integer(data, "score"),
        }),
        "PaymentSent" => Action::PaymentSent(ActionPaymentSent {
            currency: string(data, "currency").into(),
            amount: integer(data, "total_amount"),
            recurring_init: boolean(data, "recurring_init"),
            recurring_used: boolean(data, "recurring_used"),
        }),
        "PhoneCall" => {
            let name = kind(&data["reason"])
                .trim_start_matches("PhoneCallDiscardReason")
                .trim_start_matches("phoneCallDiscardReason");
            let state = serde_json::from_value(Value::String(name.into()))
                .unwrap_or(PhoneCallState::Unknown);
            Action::PhoneCall(ActionPhoneCall {
                duration: integer(data, "duration"),
                state,
                ..Default::default()
            })
        }
        "ConferenceCall" => Action::PhoneCall(ActionPhoneCall {
            conference_id: integer(data, "call_id"),
            duration: integer(data, "duration"),
            state: if boolean(data, "missed") {
                PhoneCallState::Missed
            } else if boolean(data, "active") {
                PhoneCallState::Active
            } else if integer(data, "duration") != 0 {
                PhoneCallState::Hangup
            } else {
                PhoneCallState::Invitation
            },
        }),
        "ScreenshotTaken" => Action::ScreenshotTaken(ActionScreenshotTaken {}),
        "CustomAction" => Action::CustomAction(ActionCustomAction {
            message: string(data, "message").into(),
        }),
        "BotAllowed" => {
            let mut result = ActionBotAllowed {
                attach_menu: boolean(data, "attach_menu"),
                from_request: boolean(data, "from_request"),
                domain: string(data, "domain").into(),
                ..Default::default()
            };
            if type_is(&data["app"], "BotApp") {
                result.app_id = integer(&data["app"], "id");
                result.app = string(&data["app"], "title").into();
            }
            Action::BotAllowed(result)
        }
        "SecureValuesSent" => Action::SecureValuesSent(ActionSecureValuesSent {
            types: array(data, "types")
                .iter()
                .filter_map(|t| {
                    serde_json::from_value(Value::String(
                        kind(t)
                            .trim_start_matches("SecureValueType")
                            .trim_start_matches("secureValueType")
                            .into(),
                    ))
                    .ok()
                })
                .collect(),
        }),
        "ContactSignUp" => Action::ContactSignUp(ActionContactSignUp {}),
        "GeoProximityReached" => {
            let from_id = parse_peer_id(&data["from_id"]);
            let to_id = parse_peer_id(&data["to_id"]);
            Action::GeoProximityReached(ActionGeoProximityReached {
                from_id,
                to_id,
                distance: integer(data, "distance"),
                from_self: from_id == context.self_peer_id,
                to_self: to_id == context.self_peer_id,
            })
        }
        "GroupCall" => Action::GroupCall(ActionGroupCall {
            duration: integer(data, "duration"),
        }),
        "InviteToGroupCall" => Action::InviteToGroupCall(ActionInviteToGroupCall {
            user_ids: ids(data, "users"),
        }),
        "SetMessagesTTL" => Action::SetMessagesTTL(ActionSetMessagesTTL {
            period: integer(data, "period"),
        }),
        "GroupCallScheduled" => Action::GroupCallScheduled(ActionGroupCallScheduled {
            date: integer(data, "schedule_date"),
        }),
        "SetChatTheme" => {
            if type_is(&data["theme"], "ChatTheme") {
                Action::SetChatTheme(ActionSetChatTheme {
                    emoji: string(&data["theme"], "emoticon").into(),
                })
            } else if type_is(&data["theme"], "ChatThemeUniqueGift") {
                Action::SetChatTheme(ActionSetChatTheme::default())
            } else {
                return Ok(ServiceAction::default());
            }
        }
        "ChatJoinedByRequest" => Action::ChatJoinedByRequest(ActionChatJoinedByRequest {}),
        "ChatJoinedViaCommunity" => Action::ChatJoinedViaCommunity(ActionChatJoinedViaCommunity {
            community_id: integer(data, "community_id"),
        }),
        "WebViewDataSent" => Action::WebViewDataSent(ActionWebViewDataSent {
            text: string(data, "text").into(),
        }),
        "GiftPremium" => Action::GiftPremium(ActionGiftPremium {
            cost: fill_amount_and_currency(
                integer(data, "amount"),
                string(data, "currency"),
                false,
            ),
            days: integer(data, "days"),
        }),
        "TopicCreate" => Action::TopicCreate(ActionTopicCreate {
            title: string(data, "title").into(),
        }),
        "TopicEdit" => Action::TopicEdit(ActionTopicEdit {
            title: string(data, "title").into(),
            icon_emoji_id: data["icon_emoji_id"].as_i64(),
        }),
        "SuggestProfilePhoto" => Action::SuggestProfilePhoto(ActionSuggestProfilePhoto {
            photo: photo(context, data, folder, date, dates)?,
        }),
        "SetChatWallPaper" => Action::SetChatWallPaper(ActionSetChatWallPaper {
            same: boolean(data, "same"),
            both: boolean(data, "for_both"),
        }),
        "RequestedPeer" => Action::RequestedPeer(ActionRequestedPeer {
            peers: array(data, "peers").iter().map(parse_peer_id).collect(),
            button_id: integer(data, "button_id"),
        }),
        "GiftCode" => Action::GiftCode(ActionGiftCode {
            code: string(data, "slug").into(),
            boost_peer_id: parse_peer_id(&data["boost_peer"]),
            days: integer(data, "days"),
            via_giveaway: boolean(data, "via_giveaway"),
            unclaimed: boolean(data, "unclaimed"),
        }),
        "GiveawayLaunch" => Action::GiveawayLaunch(ActionGiveawayLaunch {}),
        "GiveawayResults" => Action::GiveawayResults(ActionGiveawayResults {
            winners: integer(data, "winners_count"),
            unclaimed: integer(data, "unclaimed_count"),
            credits: boolean(data, "stars"),
        }),
        "BoostApply" => Action::BoostApply(ActionBoostApply {
            boosts: integer(data, "boosts"),
        }),
        "PaymentRefunded" => Action::PaymentRefunded(ActionPaymentRefunded {
            peer_id: parse_peer_id(&data["peer"]),
            currency: string(data, "currency").into(),
            amount: integer(data, "total_amount"),
            transaction_id: string(&data["charge"], "id").into(),
        }),
        "GiftStars" => Action::GiftCredits(ActionGiftCredits {
            cost: fill_amount_and_currency(
                integer(data, "amount"),
                string(data, "currency"),
                false,
            ),
            amount: CreditsAmount::make(integer(data, "stars"), 0, CreditsType::Stars),
        }),
        "GiftTon" => {
            let amount = integer(data, "amount");
            let unsigned = amount as u64;
            Action::GiftCredits(ActionGiftCredits {
                cost: fill_amount_and_currency(amount, string(data, "currency"), false),
                amount: CreditsAmount::make(
                    (unsigned / 1_000_000_000) as i64,
                    (unsigned % 1_000_000_000) as i64,
                    CreditsType::Ton,
                ),
            })
        }
        "PrizeStars" => Action::PrizeStars(ActionPrizeStars {
            peer_id: parse_peer_id(&data["boost_peer"]),
            amount: integer(data, "stars"),
            transaction_id: string(data, "transaction_id").into(),
            giveaway_msg_id: integer(data, "giveaway_msg_id"),
            is_unclaimed: boolean(data, "unclaimed"),
        }),
        "StarGift" => {
            let mut gift = parse_star_gift(&data["gift"]);
            gift.text = parse_text_with_entities(&data["message"]);
            gift.anonymous = boolean(data, "name_hidden");
            Action::StarGift(gift)
        }
        "StarGiftUnique" => Action::StarGift(parse_star_gift(&data["gift"])),
        "StarGiftPurchaseOffer" | "StarGiftPurchaseOfferDeclined" => {
            let mut gift = parse_star_gift(&data["gift"]);
            gift.offer = true;
            gift.offer_price = credits_amount_from_tl(&data["price"]);
            if name == "StarGiftPurchaseOffer" {
                gift.offer_expire_at = integer(data, "expires_at");
                gift.offer_accepted = boolean(data, "accepted");
                gift.offer_declined = boolean(data, "declined");
            } else {
                gift.offer_declined = true;
                gift.offer_expired = boolean(data, "expired");
            }
            Action::StarGift(gift)
        }
        "PaidMessagesRefunded" => Action::PaidMessagesRefunded(ActionPaidMessagesRefunded {
            messages: integer(data, "count"),
            stars: integer(data, "stars"),
        }),
        "PaidMessagesPrice" => Action::PaidMessagesPrice(ActionPaidMessagesPrice {
            stars: integer(data, "stars"),
            broadcast_allowed: boolean(data, "broadcast_messages_allowed"),
        }),
        "TodoCompletions" => Action::TodoCompletions(ActionTodoCompletions {
            completed: ids(data, "completed"),
            incompleted: ids(data, "incompleted"),
        }),
        "TodoAppendTasks" => Action::TodoAppendTasks(ActionTodoAppendTasks {
            items: array(data, "list")
                .iter()
                .map(parse_todo_list_item)
                .collect(),
        }),
        "PollAppendAnswer" => Action::PollAppendAnswer(ActionPollAppendAnswer::default()),
        "PollDeleteAnswer" => Action::PollDeleteAnswer(ActionPollDeleteAnswer {
            option: if type_is(&data["answer"], "PollAnswer") {
                string(&data["answer"]["text"], "text").into()
            } else {
                String::new()
            },
        }),
        "SuggestedPostApproval" => Action::SuggestedPostApproval(ActionSuggestedPostApproval {
            reject_comment: string(data, "reject_comment").into(),
            schedule_date: integer(data, "schedule_date"),
            price: credits_amount_from_tl(&data["price"]),
            rejected: boolean(data, "rejected"),
            balance_too_low: boolean(data, "balance_too_low"),
        }),
        "SuggestedPostSuccess" => Action::SuggestedPostSuccess(ActionSuggestedPostSuccess {
            price: credits_amount_from_tl(&data["price"]),
        }),
        "SuggestedPostRefund" => Action::SuggestedPostRefund(ActionSuggestedPostRefund {
            payer_initiated: boolean(data, "payer_initiated"),
        }),
        "SuggestBirthday" => Action::SuggestBirthday(ActionSuggestBirthday {
            birthday: Birthday::make(
                integer(&data["birthday"], "day") as i32,
                integer(&data["birthday"], "month") as i32,
                integer(&data["birthday"], "year") as i32,
            ),
        }),
        "NewCreatorPending" => Action::NewCreatorPending(ActionNewCreatorPending {
            new_creator_id: integer(data, "new_creator_id"),
        }),
        "ChangeCreator" => Action::ChangeCreator(ActionChangeCreator {
            new_creator_id: integer(data, "new_creator_id"),
        }),
        "NoForwardsToggle" => Action::NoForwardsToggle(ActionNoForwardsToggle {
            new_value: boolean(data, "new_value"),
        }),
        "NoForwardsRequest" => Action::NoForwardsRequest(ActionNoForwardsRequest {
            expired: boolean(data, "expired"),
            new_value: boolean(data, "new_value"),
        }),
        "ManagedBotCreated" => Action::ManagedBotCreated(ActionManagedBotCreated {
            bot_id: integer(data, "bot_id"),
        }),
        "GramTransfer" => Action::GramTransfer(ActionGramTransfer {
            amount: integer(data, "amount"),
            peer_address: string(data, "peer_address").into(),
            transaction_id: string(data, "transaction_id").into(),
            comment: string(data, "comment").into(),
            comment_encrypted: boolean(data, "comment_encrypted"),
        }),
        "WalletTonConnectRequest" => {
            Action::WalletTonConnectRequest(ActionWalletTonConnectRequest {
                session_id: data["session_id"]
                    .as_u64()
                    .unwrap_or_else(|| integer(data, "session_id") as u64),
                expires: integer(data, "expires"),
                topic: string(data, "topic").into(),
                trace_id: string(data, "trace_id").into(),
                accepted: boolean(data, "accepted"),
                declined: boolean(data, "declined"),
            })
        }
        "NoPaidMessagesTokenUsed" => {
            Action::NoPaidMessagesTokenUsed(ActionNoPaidMessagesTokenUsed {})
        }
        _ => return Ok(ServiceAction::default()),
    };
    Ok(ServiceAction {
        content: Some(content),
    })
}
