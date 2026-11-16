//! Complete Desktop service-action JSON rendering.
use super::{
    error::Result, json_serialize::*, json_values::MessageValues, model::*, model_actions::*,
    model_media::TodoListItem,
};
fn actor(v: &mut MessageValues<'_>, name: &str) {
    v.push_actor();
    v.push_action(name);
}
fn price(v: &mut MessageValues<'_>, amount: &CreditsAmount) {
    v.string("price_amount_whole", number(amount.whole()));
    v.string("price_amount_nano", number(amount.nano()));
    v.string("price_currency", if amount.ton() { "TON" } else { "Stars" });
}
pub fn serialize_todo_items(context: &mut JsonContext, items: &[TodoListItem]) -> Vec<u8> {
    let mut values = Vec::new();
    for item in items {
        context.nesting.push(K_ARRAY);
        let text = serialize_text(context, &item.text, false);
        values.push(serialize_object(
            context,
            &[("text", text), ("id", number(item.id))],
        ));
        context.nesting.pop();
    }
    serialize_array(context, &values)
}
pub fn push_service_action(v: &mut MessageValues<'_>, data: Option<&Action>) -> Result<()> {
    let Some(data) = data else {
        return Ok(());
    };
    use Action as A;
    match data {
        A::ChatDeletePhoto(_) => actor(v, "delete_group_photo"),
        A::ChatMigrateTo(_) => actor(v, "migrate_to_supergroup"),
        A::HistoryClear(_) => actor(v, "clear_history"),
        A::ScreenshotTaken(_) => actor(v, "take_screenshot"),
        A::ContactSignUp(_) => actor(v, "joined_telegram"),
        A::PhoneNumberRequest(_) => actor(v, "requested_phone_number"),
        A::ChatJoinedByRequest(_) => actor(v, "join_group_by_request"),
        A::ChatEditTitle(d) => {
            actor(v, "edit_group_title");
            v.string("title", &d.title);
        }
        A::ChannelCreate(d) => {
            actor(v, "create_channel");
            v.string("title", &d.title);
        }
        A::ChannelMigrateFrom(d) => {
            actor(v, "migrate_from_group");
            v.string("title", &d.title);
        }
        A::TopicCreate(d) => {
            actor(v, "topic_created");
            v.string("title", &d.title);
        }
        A::ChatCreate(d) => {
            actor(v, "create_group");
            v.string("title", &d.title);
            v.push_user_names(&d.user_ids, "members");
        }
        A::ChatEditPhoto(d) => {
            actor(v, "edit_group_photo");
            v.push_photo(&d.photo.image)?;
            v.push_spoiler(d.photo.spoilered);
        }
        A::SuggestProfilePhoto(d) => {
            actor(v, "suggest_profile_photo");
            v.push_photo(&d.photo.image)?;
            v.push_spoiler(d.photo.spoilered);
        }
        A::ChatAddUser(d) => {
            actor(v, "invite_members");
            v.push_user_names(&d.user_ids, "members");
        }
        A::InviteToGroupCall(d) => {
            actor(v, "invite_to_group_call");
            v.push_user_names(&d.user_ids, "members");
        }
        A::ChatDeleteUser(d) => {
            actor(v, "remove_members");
            v.push_user_names(&[d.user_id], "members");
        }
        A::ChatJoinedByLink(d) => {
            actor(v, "join_group_by_link");
            v.bare("inviter", v.wrap_user_name(d.inviter_id));
        }
        A::PinMessage(_) => {
            actor(v, "pin_message");
            v.push_reply_to_msg_id("message_id");
        }
        A::GameScore(d) => {
            actor(v, "score_in_game");
            v.push_reply_to_msg_id("game_message_id");
            v.number("score", d.score);
        }
        A::PaymentSent(d) => {
            v.push_action("send_payment");
            v.number("amount", d.amount);
            v.string("currency", &d.currency);
            v.push_reply_to_msg_id("invoice_message_id");
            if d.recurring_used {
                v.string("recurring", "used");
            } else if d.recurring_init {
                v.string("recurring", "init");
            }
        }
        A::PhoneCall(d) => {
            actor(
                v,
                if d.conference_id != 0 {
                    "conference_call"
                } else {
                    "phone_call"
                },
            );
            if d.duration != 0 {
                v.number("duration_seconds", d.duration);
            }
            if d.conference_id != 0 {
                v.boolean("is_active", d.state == PhoneCallState::Active);
                v.boolean("is_missed", d.state == PhoneCallState::Missed);
            } else {
                v.string(
                    "discard_reason",
                    match d.state {
                        PhoneCallState::Busy => "busy",
                        PhoneCallState::Disconnect => "disconnect",
                        PhoneCallState::Hangup => "hangup",
                        PhoneCallState::Missed => "missed",
                        PhoneCallState::MigrateConferenceCall => "migrate_conference_all",
                        _ => "",
                    },
                );
            }
        }
        A::CustomAction(d) => {
            v.push_actor();
            v.string("information_text", &d.message);
        }
        A::BotAllowed(d) => {
            if d.attach_menu {
                v.push_action("attach_menu_bot_allowed");
            } else if d.from_request {
                v.push_action("web_app_bot_allowed");
            } else if d.app_id != 0 {
                v.push_action("allow_sending_messages");
                v.number("reason_app_id", d.app_id as u64);
                v.string("reason_app_name", &d.app);
            } else {
                v.push_action("allow_sending_messages");
                v.string("reason_domain", &d.domain);
            }
        }
        A::SecureValuesSent(d) => {
            v.push_action("send_passport_values");
            let names = d
                .types
                .iter()
                .map(|one| {
                    serialize_string(match one {
                        SecureValueType::PersonalDetails => "personal_details",
                        SecureValueType::Passport => "passport",
                        SecureValueType::DriverLicense => "driver_license",
                        SecureValueType::IdentityCard => "identity_card",
                        SecureValueType::InternalPassport => "internal_passport",
                        SecureValueType::Address => "address_information",
                        SecureValueType::UtilityBill => "utility_bill",
                        SecureValueType::BankStatement => "bank_statement",
                        SecureValueType::RentalAgreement => "rental_agreement",
                        SecureValueType::PassportRegistration => "passport_registration",
                        SecureValueType::TemporaryRegistration => "temporary_registration",
                        SecureValueType::Phone => "phone_number",
                        SecureValueType::Email => "email",
                    })
                })
                .collect::<Vec<_>>();
            v.bare("values", serialize_array(v.context, &names));
        }
        A::GeoProximityReached(d) => {
            v.push_action("proximity_reached");
            if d.from_id != 0 {
                v.bare("from", v.wrap_peer_name(d.from_id));
                v.peer("from_id", d.from_id);
            }
            if d.to_id != 0 {
                v.bare("to", v.wrap_peer_name(d.to_id));
                v.peer("to_id", d.to_id);
            }
            v.number("distance", d.distance);
        }
        A::GroupCall(d) => {
            actor(v, "group_call");
            if d.duration != 0 {
                v.number("duration", d.duration);
            }
        }
        A::SetMessagesTTL(d) => {
            actor(v, "set_messages_ttl");
            v.number("period", d.period);
        }
        A::GroupCallScheduled(d) => {
            actor(v, "group_call_scheduled");
            v.number("schedule_date", d.date);
        }
        A::SetChatTheme(d) => {
            actor(v, "edit_chat_theme");
            v.string("emoticon", &d.emoji);
        }
        A::ChatJoinedViaCommunity(d) => {
            actor(v, "join_group_via_community");
            v.number("community_id", d.community_id);
        }
        A::WebViewDataSent(d) => {
            v.push_action("send_webview_data");
            v.string("text", &d.text);
        }
        A::GiftPremium(d) => {
            actor(v, "send_premium_gift");
            v.string("cost", &d.cost);
            if d.days != 0 {
                v.number("days", d.days);
            }
        }
        A::TopicEdit(d) => {
            actor(v, "topic_edit");
            v.string("new_title", &d.title);
            if let Some(id) = d.icon_emoji_id {
                v.number("new_icon_emoji_id", id as u64);
            }
        }
        A::RequestedPeer(d) => {
            actor(v, "requested_peer");
            v.number("button_id", d.button_id);
            v.string(
                "peers",
                serialize_array(
                    v.context,
                    &d.peers.iter().map(|id| number(*id)).collect::<Vec<_>>(),
                ),
            );
        }
        A::GiftCode(d) => {
            v.push_action("gift_code_prize");
            v.string("gift_code", &d.code);
            if d.boost_peer_id != 0 {
                v.peer("boost_peer_id", d.boost_peer_id);
            }
            v.number("days", d.days);
            v.boolean("unclaimed", d.unclaimed);
            v.boolean("via_giveaway", d.via_giveaway);
        }
        A::GiveawayLaunch(_) => v.push_action("giveaway_launch"),
        A::GiveawayResults(d) => {
            v.push_action("giveaway_results");
            v.number("winners", d.winners);
            v.number("unclaimed", d.unclaimed);
            v.boolean("stars", d.credits);
        }
        A::SetChatWallPaper(d) => {
            actor(
                v,
                if d.same {
                    "set_same_chat_wallpaper"
                } else {
                    "set_chat_wallpaper"
                },
            );
            v.push_reply_to_msg_id("message_id");
        }
        A::BoostApply(d) => {
            actor(v, "boost_apply");
            v.number("boosts", d.boosts);
        }
        A::PaymentRefunded(d) => {
            v.push_action("refunded_payment");
            v.number("amount", d.amount);
            v.string("currency", &d.currency);
            v.bare("peer_name", v.wrap_peer_name(d.peer_id));
            v.peer("peer_id", d.peer_id);
            v.string("charge_id", &d.transaction_id);
        }
        A::GiftCredits(d) => {
            actor(
                v,
                if d.amount.ton() {
                    "send_ton_gift"
                } else {
                    "send_stars_gift"
                },
            );
            v.string("cost", &d.cost);
            if !d.amount.empty() {
                v.number("amount_whole", d.amount.whole());
                v.number("amount_nano", d.amount.nano());
            }
        }
        A::PrizeStars(d) => {
            actor(v, "stars_prize");
            v.peer("boost_peer_id", d.peer_id);
            v.bare("boost_peer_name", v.wrap_peer_name(d.peer_id));
            v.number("stars", d.amount);
            v.boolean("is_unclaimed", d.is_unclaimed);
            v.number("giveaway_msg_id", d.giveaway_msg_id);
            v.string("transaction_id", &d.transaction_id);
        }
        A::StarGift(d) => {
            actor(v, "send_star_gift");
            v.number("gift_id", d.gift_id as u64);
            v.number("stars", d.stars);
            v.boolean("is_limited", d.limited);
            v.boolean("is_anonymous", d.anonymous);
            let text = serialize_text(v.context, &d.text, false);
            v.bare("gift_text", text);
        }
        A::PaidMessagesRefunded(d) => {
            actor(v, "paid_messages_refund");
            v.number("messages_count", d.messages);
            v.number("stars_count", d.stars);
        }
        A::PaidMessagesPrice(d) => {
            actor(v, "paid_messages_price_change");
            v.number("price_stars", d.stars);
            v.boolean("is_broadcast_messages_allowed", d.broadcast_allowed);
        }
        A::TodoCompletions(d) => {
            actor(v, "todo_completions");
            let list = |ids: &[i64]| {
                format!(
                    "[{}]",
                    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
                )
                .into_bytes()
            };
            v.bare("completed", list(&d.completed));
            v.bare("incompleted", list(&d.incompleted));
        }
        A::TodoAppendTasks(d) => {
            actor(v, "todo_append_tasks");
            let items = serialize_todo_items(v.context, &d.items);
            v.bare("items", items);
        }
        A::PollAppendAnswer(d) => {
            actor(v, "poll_append_answer");
            v.string("option", &d.option);
        }
        A::PollDeleteAnswer(d) => {
            actor(v, "poll_delete_answer");
            v.string("option", &d.option);
        }
        A::SuggestedPostApproval(d) => {
            actor(v, "process_suggested_post");
            if d.rejected {
                v.boolean("rejected", true);
                v.string("comment", &d.reject_comment);
            } else {
                price(v, &d.price);
                v.number("scheduled_date", d.schedule_date);
            }
        }
        A::SuggestedPostSuccess(d) => {
            actor(v, "suggested_post_success");
            price(v, &d.price);
        }
        A::SuggestedPostRefund(d) => {
            actor(v, "suggested_post_refund");
            v.boolean("user_initiated", d.payer_initiated);
        }
        A::SuggestBirthday(d) => {
            actor(v, "suggest_birthday");
            v.number("day", d.birthday.day());
            v.number("month", d.birthday.month());
            if d.birthday.year() != 0 {
                v.number("year", d.birthday.year());
            }
        }
        A::NoForwardsToggle(d) => {
            actor(v, "no_forwards_toggle");
            v.boolean("new_value", d.new_value);
        }
        A::NoForwardsRequest(d) => {
            actor(v, "no_forwards_request");
            v.boolean("expired", d.expired);
            v.boolean("new_value", d.new_value);
        }
        A::NewCreatorPending(d) => {
            actor(v, "new_creator_pending");
            v.bare("new_creator", v.wrap_user_name(d.new_creator_id));
        }
        A::ChangeCreator(d) => {
            actor(v, "change_creator");
            v.bare("new_creator", v.wrap_user_name(d.new_creator_id));
        }
        A::ManagedBotCreated(d) => {
            actor(v, "managed_bot_created");
            v.bare("bot", v.wrap_user_name(d.bot_id));
        }
        A::GramTransfer(d) => {
            actor(v, "gram_transfer");
            v.number("amount_whole", d.amount / 1_000_000_000);
            v.number("amount_nano", d.amount % 1_000_000_000);
            v.string("peer_address", &d.peer_address);
            v.string("transaction_id", &d.transaction_id);
            v.boolean("comment_encrypted", d.comment_encrypted);
            v.string(
                if d.comment_encrypted {
                    "encrypted_comment_base64"
                } else {
                    "comment"
                },
                &d.comment,
            );
        }
        A::WalletTonConnectRequest(d) => {
            actor(v, "ton_connect_request");
            v.number("session_id", d.session_id);
            v.number("expires", d.expires);
            v.string("topic", &d.topic);
            v.string("trace_id", &d.trace_id);
            v.boolean("accepted", d.accepted);
            v.boolean("declined", d.declined);
        }
        A::NoPaidMessagesTokenUsed(_) => actor(v, "no_paid_messages_token_used"),
    }
    Ok(())
}
