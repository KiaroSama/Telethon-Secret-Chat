//! Desktop English service sentences, including preserved action-specific quirks.
use super::{
    error::Result,
    format::{DateContext, fill_amount_and_currency, format_date_time, format_grams_amount},
    html_text::{
        PeersMap, format_date_text, format_text, format_time_text, month_name, serialize_list,
        serialize_string,
    },
    model::peer_from_user,
    model_actions::*,
    model_dialogs::{DialogInfo, DialogType},
    model_message::Message,
};
pub fn qt_number(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".into()
        } else if value.is_sign_negative() {
            "-inf".into()
        } else {
            "inf".into()
        };
    }
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent = exponent.parse::<i32>().unwrap_or(0);
    let trim = |s: &str| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s.to_owned()
        }
    };
    if !(-4..6).contains(&exponent) {
        format!(
            "{}e{}{:02}",
            trim(mantissa),
            if exponent < 0 { '-' } else { '+' },
            exponent.unsigned_abs()
        )
    } else {
        trim(&format!(
            "{value:.precision$}",
            precision = (5 - exponent).max(0) as usize
        ))
    }
}
fn quoted(title: &str) -> String {
    format!("&laquo;{}&raquo;", serialize_string(title))
}
fn days(value: i64) -> String {
    format!("{value} {}", if value > 1 { "days" } else { "day" })
}
fn distance(meters: i64) -> String {
    if meters >= 1000 {
        format!("{} km", qt_number((10 * (meters / 10)) as f64 / 1000.0))
    } else if meters == 1 {
        "1 meter".into()
    } else {
        format!("{meters} meters")
    }
}
fn todo_list(values: &[i64]) -> String {
    serialize_list(&values.iter().map(i64::to_string).collect::<Vec<_>>())
}
pub fn service_text(
    message: &Message,
    dialog: &DialogInfo,
    peers: &PeersMap,
    domain: &str,
    base: &str,
    mut link: impl FnMut(&str) -> String,
    dates: &DateContext,
) -> Result<Vec<u8>> {
    let from = peers.wrap_peer_name(message.from_id);
    let channel = matches!(
        dialog.r#type,
        DialogType::PrivateChannel | DialogType::PublicChannel
    );
    let Some(data) = &message.action.content else {
        return Ok(vec![]);
    };
    use Action as A;
    let result = match data {
        A::ChatCreate(d) => format!(
            "{from} created group {}{}",
            quoted(&d.title),
            if d.user_ids.is_empty() {
                String::new()
            } else {
                format!(" with members {}", peers.wrap_user_names(&d.user_ids))
            }
        ),
        A::ChatEditTitle(d) => {
            if channel {
                format!("Channel title changed to {}", quoted(&d.title))
            } else {
                format!("{from} changed group title to {}", quoted(&d.title))
            }
        }
        A::ChatEditPhoto(_) => {
            if channel {
                "Channel photo changed".into()
            } else {
                format!("{from} changed group photo")
            }
        }
        A::ChatDeletePhoto(_) => {
            if channel {
                "Channel photo removed".into()
            } else {
                format!("{from} removed group photo")
            }
        }
        A::ChatAddUser(d) => format!("{from} invited {}", peers.wrap_user_names(&d.user_ids)),
        A::ChatDeleteUser(d) => format!("{from} removed {}", peers.wrap_user_name(d.user_id)),
        A::ChatJoinedByLink(d) => format!(
            "{from} joined group by link from {}",
            peers.wrap_user_name(d.inviter_id)
        ),
        A::ChannelCreate(d) => format!("Channel {} created", quoted(&d.title)),
        A::ChatMigrateTo(_) => format!("{from} converted this group to a supergroup"),
        A::ChannelMigrateFrom(d) => format!(
            "{from} converted a basic group to this supergroup {}",
            quoted(&d.title)
        ),
        A::PinMessage(_) => format!("{from} pinned {}", link("this message")),
        A::HistoryClear(_) => "History cleared".into(),
        A::GameScore(d) => format!("{from} scored {} in {}", d.score, link("this game")),
        A::PaymentSent(d) => {
            let amount = fill_amount_and_currency(d.amount, &d.currency, false);
            if d.recurring_used {
                format!("You were charged {amount} via recurring payment")
            } else {
                format!(
                    "You have successfully transferred {amount} for {}{}",
                    link("this invoice"),
                    if d.recurring_init {
                        " and allowed future recurring payments"
                    } else {
                        ""
                    }
                )
            }
        }
        A::ScreenshotTaken(_) => format!("{from} took a screenshot"),
        A::CustomAction(d) => serialize_string(&d.message),
        A::BotAllowed(d) => {
            let reason = if d.attach_menu {
                "when you added it in the attachment menu.".into()
            } else if d.from_request {
                "in his web-app.".into()
            } else if d.app.is_empty() {
                format!("when you opened {}", serialize_string(&d.app))
            } else {
                format!("when you logged in on {}", serialize_string(&d.domain))
            };
            format!("You allowed this bot to message you {reason}")
        }
        A::SecureValuesSent(d) => {
            let names = d
                .types
                .iter()
                .map(|kind| {
                    match kind {
                        SecureValueType::PersonalDetails => "Personal details",
                        SecureValueType::Passport => "Passport",
                        SecureValueType::DriverLicense => "Driver license",
                        SecureValueType::IdentityCard => "Identity card",
                        SecureValueType::InternalPassport => "Internal passport",
                        SecureValueType::Address => "Address information",
                        SecureValueType::UtilityBill => "Utility bill",
                        SecureValueType::BankStatement => "Bank statement",
                        SecureValueType::RentalAgreement => "Rental agreement",
                        SecureValueType::PassportRegistration => "Passport registration",
                        SecureValueType::TemporaryRegistration => "Temporary registration",
                        SecureValueType::Phone => "Phone number",
                        SecureValueType::Email => "Email",
                    }
                    .to_owned()
                })
                .collect::<Vec<_>>();
            format!(
                "You have sent the following documents: {}",
                serialize_list(&names)
            )
        }
        A::ContactSignUp(_) => format!("{from} joined Telegram"),
        A::GeoProximityReached(d) => {
            let from = peers.wrap_peer_name(d.from_id);
            let to = peers.wrap_peer_name(d.to_id);
            let distance = distance(d.distance);
            if d.from_self {
                format!("You are now within {distance} from {to}")
            } else if d.to_self {
                format!("{from} is now within {distance} from you")
            } else {
                format!("{from} is now within {distance} from {to}")
            }
        }
        A::PhoneNumberRequest(_) => format!("{from} requested your phone number"),
        A::GroupCall(d) => {
            let duration = if d.duration != 0 {
                format!(" ({} seconds)", d.duration)
            } else {
                String::new()
            };
            if channel {
                format!("Voice chat{duration}")
            } else {
                format!("{from} started voice chat{duration}")
            }
        }
        A::InviteToGroupCall(d) => format!(
            "{from} invited {} to the voice chat",
            peers.wrap_user_names(&d.user_ids)
        ),
        A::SetMessagesTTL(d) => {
            let period = match d.period {
                604800 => "7 days".into(),
                86400 => "24 hours".into(),
                value if value > 0 => {
                    format!("{value} {}", if value == 1 { "second" } else { "seconds" })
                }
                _ => String::new(),
            };
            if channel {
                if d.period != 0 {
                    format!("New messages will auto-delete in {period}")
                } else {
                    "New messages will not auto-delete".into()
                }
            } else if d.period != 0 {
                format!("{from} has set messages to auto-delete in {period}")
            } else {
                format!("{from} has set messages not to auto-delete")
            }
        }
        A::GroupCallScheduled(d) => {
            let date = format_date_time(d.date, false, ".", ":", " ", dates)?;
            if channel {
                format!("Voice chat scheduled for {date}")
            } else {
                format!("{from} scheduled a voice chat for {date}")
            }
        }
        A::SetChatTheme(d) => {
            if d.emoji.is_empty() {
                if channel {
                    "Channel theme was disabled".into()
                } else {
                    format!("{from} disabled chat theme")
                }
            } else if channel {
                format!(
                    "Channel theme was changed to {}",
                    serialize_string(&d.emoji)
                )
            } else {
                format!(
                    "{from} changed chat theme to {}",
                    serialize_string(&d.emoji)
                )
            }
        }
        A::ChatJoinedByRequest(_) => format!("{from} joined group by request"),
        A::ChatJoinedViaCommunity(_) => format!("{from} joined group via a community"),
        A::WebViewDataSent(d) => format!(
            "You have just successfully transferred data from the {} button to the bot",
            quoted(&d.text)
        ),
        A::GiftPremium(d) => {
            if d.days == 0 || d.cost.is_empty() {
                format!("{from} sent you a gift.")
            } else {
                format!(
                    "{from} sent you a gift for {}: Telegram Premium for {} days.",
                    serialize_string(&d.cost),
                    d.days
                )
            }
        }
        A::TopicCreate(d) => format!("{from} created topic {}", quoted(&d.title)),
        A::TopicEdit(d) => {
            let mut parts = vec![];
            if !d.title.is_empty() {
                parts.push(format!("title to {}", quoted(&d.title)));
            }
            if let Some(icon) = d.icon_emoji_id {
                parts.push(format!("icon to &laquo;{icon}&raquo;"));
            }
            format!("{from} changed topic {}", parts.join(","))
        }
        A::SuggestProfilePhoto(_) => format!("{from} suggests to use this photo"),
        A::RequestedPeer(_) => "requested: ".into(),
        A::SetChatWallPaper(d) => {
            if d.same {
                format!("{from} set {} for this chat", link("the same background"))
            } else {
                format!("{from} set a new background for this chat")
            }
        }
        A::GiftCode(d) => {
            if d.unclaimed {
                format!(
                    "This is an unclaimed Telegram Premium for {} prize in a giveaway organized by a channel.",
                    days(d.days)
                )
            } else if d.via_giveaway {
                format!(
                    "You won a Telegram Premium for {} prize in a giveaway organized by a channel.",
                    days(d.days)
                )
            } else {
                format!(
                    "You've received a Telegram Premium for {} gift from a channel.",
                    days(d.days)
                )
            }
        }
        A::GiveawayLaunch(_) => format!(
            "{from} just started a giveaway of Telegram Premium subscriptions to its followers."
        ),
        A::GiveawayResults(d) => {
            let suffix = if d.credits {
                "their prize."
            } else {
                "private messages with giftcodes."
            };
            if d.winners == 0 {
                "No winners of the giveaway could be selected.".into()
            } else if d.unclaimed != 0 {
                format!(
                    "Some winners of the giveaway were randomly selected by Telegram and received {suffix}"
                )
            } else {
                format!(
                    "{} of the giveaway was randomly selected by Telegram and received {suffix}",
                    d.winners
                )
            }
        }
        A::BoostApply(d) => format!(
            "{from} boosted the group {} {}",
            d.boosts,
            if d.boosts > 1 { "times" } else { "time" }
        ),
        A::PaymentRefunded(d) => format!(
            "{} refunded back {}",
            peers.wrap_peer_name(d.peer_id),
            fill_amount_and_currency(d.amount, &d.currency, false)
        ),
        A::GiftCredits(d) => {
            if d.amount.empty() || d.cost.is_empty() {
                format!("{from} sent you a gift.")
            } else {
                format!(
                    "{from} sent you a gift for {}: {}{}",
                    serialize_string(&d.cost),
                    qt_number(d.amount.value()),
                    if d.amount.ton() {
                        " TON."
                    } else {
                        " Telegram Stars."
                    }
                )
            }
        }
        A::PrizeStars(d) => format!(
            "You won a prize in a giveaway organized by {}.\n Your prize is {} Telegram Stars.",
            peers.wrap_peer_name(d.peer_id),
            d.amount
        ),
        A::StarGift(d) => format!("{from} sent you a gift of {} Telegram Stars.", d.stars),
        A::PaidMessagesRefunded(d) => {
            let peer = peers.wrap_peer_name(dialog.peer_id);
            if message.out {
                format!(
                    "You refunded {} Stars for {} messages to {peer}",
                    d.stars, d.messages
                )
            } else {
                format!(
                    "{peer} refunded {} Stars for {} messages to you",
                    d.stars, d.messages
                )
            }
        }
        A::PaidMessagesPrice(d) => {
            if channel {
                if !d.broadcast_allowed {
                    "Direct messages were disabled.".into()
                } else {
                    format!(
                        "Price per direct message changed to {} Telegram Stars.",
                        d.stars
                    )
                }
            } else {
                format!("Price per message changed to {} Telegram Stars.", d.stars)
            }
        }
        A::TodoCompletions(d) => {
            let complete = todo_list(&d.completed);
            let incomplete = todo_list(&d.incompleted);
            let link = link("this todo list") + ".";
            if d.completed.is_empty() && !d.incompleted.is_empty() {
                format!("{from} marked {incomplete} as not done yet in {link}")
            } else if !d.completed.is_empty() && d.incompleted.is_empty() {
                format!("{from} marked {complete} as done in {link}")
            } else {
                format!(
                    "{from} marked {complete} as done and {incomplete} as not done yet in {link}"
                )
            }
        }
        A::TodoAppendTasks(d) => {
            let mut result = format!("{from} added tasks: ").into_bytes();
            for (i, task) in d.items.iter().enumerate() {
                if i != 0 {
                    result.extend_from_slice(b", ");
                }
                result.extend_from_slice(b"&quot;");
                result.extend(format_text(&task.text, domain, base));
                result.extend_from_slice(b"&quot;");
            }
            return Ok(result);
        }
        A::PollAppendAnswer(d) => format!(
            "{from} added &quot;{}&quot; to the poll.",
            serialize_string(&d.option)
        ),
        A::PollDeleteAnswer(d) => format!(
            "{from} removed &quot;{}&quot; from the poll.",
            serialize_string(&d.option)
        ),
        A::SuggestedPostApproval(d) => {
            let mut result = format!(
                "{from} {} your suggested post",
                if d.rejected { "rejected" } else { "approved" }
            );
            if !d.price.empty() {
                result += &format!(
                    ", for {}{}",
                    qt_number(d.price.value()),
                    if d.price.ton() { " TON" } else { " stars" }
                );
            }
            if d.schedule_date != 0 {
                result += &format!(
                    ", {} at {}",
                    format_date_text(d.schedule_date, dates)?,
                    format_time_text(d.schedule_date, dates)?
                );
            }
            if d.reject_comment.is_empty() {
                result.push('.');
            } else {
                result += &format!(
                    ", with comment: &quot;{}&quot;",
                    serialize_string(&d.reject_comment)
                );
            }
            result
        }
        A::SuggestedPostSuccess(d) => format!(
            "The paid post was shown for 24 hours and {}{} were transferred to the channel.",
            qt_number(d.price.value()),
            if d.price.ton() { " TON" } else { " stars" }
        ),
        A::SuggestedPostRefund(d) => {
            if d.payer_initiated {
                "The user refunded the payment, post was deleted.".into()
            } else {
                "The admin deleted the post early, the payment was refunded.".into()
            }
        }
        A::SuggestBirthday(d) => {
            let month = d.birthday.month();
            format!(
                "{from} suggests to add a date of birth: {}{}{}",
                d.birthday.day(),
                if (1..=12).contains(&month) {
                    format!(" {}", month_name(month as i8))
                } else {
                    String::new()
                },
                if d.birthday.year() != 0 {
                    format!(" {}", d.birthday.year())
                } else {
                    String::new()
                }
            )
        }
        A::NoForwardsToggle(d) => format!(
            "{from} {} sharing in this chat",
            if d.new_value { "disabled" } else { "enabled" }
        ),
        A::NoForwardsRequest(_) => format!("{from} requested to enable sharing in this chat"),
        A::NewCreatorPending(d) => format!(
            "{} will become the new main admin in 7 days if {from} does not return",
            peers.wrap_user_name(d.new_creator_id)
        ),
        A::ChangeCreator(d) => format!(
            "{from} made {} the new main admin of the group",
            peers.wrap_user_name(d.new_creator_id)
        ),
        A::ManagedBotCreated(d) => {
            format!("{from} created a bot {}", peers.wrap_user_name(d.bot_id))
        }
        A::GramTransfer(d) => {
            let amount = format_grams_amount(d.amount);
            let address = if d.peer_address.is_empty() {
                String::new()
            } else {
                format!(" ({})", serialize_string(&d.peer_address))
            };
            let hidden = [peer_from_user(333000), peer_from_user(777000)].contains(&dialog.peer_id);
            let mut result = if message.out {
                format!(
                    "You sent {amount} to {}{address}",
                    peers.wrap_peer_name(dialog.peer_id)
                )
            } else {
                format!(
                    "{}{address} sent you {amount}",
                    if hidden { "Someone" } else { &from }
                )
            };
            if !d.transaction_id.is_empty() {
                result += &format!(", transaction {}", serialize_string(&d.transaction_id));
            }
            if d.comment_encrypted {
                result += &format!(
                    ", with an <span class=\"gram_transfer_encrypted_comment\" data-encrypted-comment=\"{}\">encrypted comment</span>",
                    serialize_string(&d.comment)
                );
            } else if !d.comment.is_empty() {
                result += &format!(
                    ", with comment: &laquo;{}&raquo;",
                    serialize_string(&d.comment)
                );
            }
            result
        }
        A::WalletTonConnectRequest(d) => {
            let topic = if d.topic.is_empty() {
                String::new()
            } else {
                format!(" &laquo;{}&raquo;", serialize_string(&d.topic))
            };
            format!(
                "You {} a TON Connect request{topic}.",
                if d.accepted {
                    "approved"
                } else if d.declined {
                    "declined"
                } else {
                    "received"
                }
            )
        }
        A::NoPaidMessagesTokenUsed(_) => format!("{from} started a chat via a free messages link"),
        A::PhoneCall(_) => String::new(),
    };
    Ok(result.into_bytes())
}
