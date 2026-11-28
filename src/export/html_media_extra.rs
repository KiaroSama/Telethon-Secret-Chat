//! Desktop poll, to-do and giveaway markup.
use super::{
    error::Result,
    format::{DateContext, format_date_time},
    html_countries::{country_name_by_iso2, flag_emoji_by_iso2},
    html_text::{PeersMap, WrapBase, format_text, serialize_string},
    model::peer_from_channel,
    model_media::*,
};
fn votes(count: i64) -> String {
    if count > 1 {
        format!("{count} votes")
    } else if count > 0 {
        format!("{count} vote")
    } else {
        "No votes".into()
    }
}
pub fn participants(
    data: &GiveawayStart,
    channel: bool,
    group: bool,
    count: usize,
) -> &'static str {
    if count == 0 || !channel && !group {
        return "";
    }
    let many = count > 1;
    if data.all {
        if group && channel {
            if many {
                "All members of the groups and channels:"
            } else {
                "All members of the group:"
            }
        } else if group {
            if many {
                "All members of the groups:"
            } else {
                "All members of the group:"
            }
        } else if many {
            "All subscribers of the channels:"
        } else {
            "All subscribers of the channel:"
        }
    } else if group && channel {
        if many {
            "All users who joined the groups and channels below after this date:"
        } else {
            "All users who joined the group below after this date:"
        }
    } else if group {
        if many {
            "All users who joined the groups below after this date:"
        } else {
            "All users who joined the group below after this date:"
        }
    } else if many {
        "All users who joined the channels below after this date:"
    } else {
        "All users who joined the channel below after this date:"
    }
}
impl WrapBase {
    pub fn push_section(&mut self, class: &str, content: &str) -> Result<String> {
        Ok(self.push_div(class, "") + content + &self.pop_tag()?)
    }
    fn push_section_bytes(&mut self, class: &str, content: &[u8]) -> Result<Vec<u8>> {
        let mut result = self.push_div(class, "").into_bytes();
        result.extend_from_slice(content);
        result.extend(self.pop_tag()?.into_bytes());
        Ok(result)
    }
    pub fn push_poll(&mut self, data: &Poll, domain: &str, base: &str) -> Result<Vec<u8>> {
        let mut result = (self.push_div("media_wrap clearfix", "")
            + &self.push_div("media_poll", ""))
            .into_bytes();
        result.extend(
            self.push_section_bytes("question bold", &format_text(&data.question, domain, base))?,
        );
        result.extend(
            self.push_section(
                "details",
                if data.closed {
                    "Final results"
                } else {
                    "Anonymous poll"
                },
            )?
            .into_bytes(),
        );
        for answer in &data.answers {
            let mut text = b"- ".to_vec();
            text.extend(format_text(&answer.text, domain, base));
            if answer.votes != 0 {
                text.extend(
                    format!(
                        " <span class=\"details\">{}{}</span>",
                        votes(answer.votes),
                        if answer.my { ", chosen vote" } else { "" }
                    )
                    .into_bytes(),
                );
            }
            result.extend(self.push_section_bytes("answer", &text)?);
        }
        result.extend(
            self.push_section("total details\t", &votes(data.total_votes))?
                .into_bytes(),
        );
        result.extend(self.pop_tag()?.into_bytes());
        result.extend(self.pop_tag()?.into_bytes());
        Ok(result)
    }
    pub fn push_todo_list(&mut self, data: &TodoList, domain: &str, base: &str) -> Result<Vec<u8>> {
        let mut result = (self.push_div("media_wrap clearfix", "")
            + &self.push_div("media_poll", ""))
            .into_bytes();
        result.extend(
            self.push_section_bytes("question bold", &format_text(&data.title, domain, base))?,
        );
        result.extend(self.push_section("details", "To-do List")?.into_bytes());
        for item in &data.items {
            let mut text = b"- ".to_vec();
            text.extend(format_text(&item.text, domain, base));
            result.extend(self.push_section_bytes("answer", &text)?);
        }
        result.extend(self.pop_tag()?.into_bytes());
        result.extend(self.pop_tag()?.into_bytes());
        Ok(result)
    }
    pub fn push_giveaway_start(
        &mut self,
        peers: &PeersMap,
        data: &GiveawayStart,
        dates: &DateContext,
    ) -> Result<String> {
        let mut result =
            self.push_div("media_wrap clearfix", "") + &self.push_div("media_giveaway", "");
        result += &self.push_section(
            "section_title bold",
            if data.quantity > 1 {
                "Giveaway Prizes"
            } else {
                "Giveaway Prize"
            },
        )?;
        result += &self.push_section(
            "section_body",
            &format!(
                "<b>{}</b> {}",
                data.quantity,
                serialize_string(&data.additional_prize)
            ),
        )?;
        result += &self.push_section("section_title bold", "with")?;
        let body = if data.credits > 0 {
            format!(
                "<b>{} {}</b> will be distributed {} <b>{}</b> {}.",
                data.credits,
                if data.credits == 1 { "Star" } else { "Stars" },
                if data.quantity == 1 { "to" } else { "among" },
                data.quantity,
                if data.quantity == 1 {
                    "winner"
                } else {
                    "winners"
                }
            )
        } else {
            format!(
                "<b>{}</b> {} for <b>{}</b> {}.",
                data.quantity,
                if data.quantity > 1 {
                    "Telegram Premium Subscriptions"
                } else {
                    "Telegram Premium Subscription"
                },
                data.months,
                if data.months > 1 { "months" } else { "month" }
            )
        };
        result += &self.push_section("section_body", &body)?;
        result += &self.push_section("section_title bold", "Participants")?;
        let (mut channel, mut group) = (false, false);
        for id in &data.channels {
            if let Some(chat) = peers.peer(peer_from_channel(*id)).chat() {
                if chat.is_broadcast {
                    channel = true;
                } else if chat.is_supergroup {
                    group = true;
                }
            }
        }
        result += &self.push_section(
            "section_body",
            &serialize_string(participants(data, channel, group, data.channels.len())),
        )?;
        let countries = data
            .countries
            .iter()
            .map(|code| {
                format!(
                    "{} {}",
                    flag_emoji_by_iso2(code),
                    country_name_by_iso2(code, &[])
                )
            })
            .collect::<Vec<_>>();
        if !countries.is_empty() {
            let mut united = countries[0].clone();
            for (i, country) in countries.iter().enumerate().skip(1) {
                united += if i + 1 == countries.len() {
                    " and "
                } else {
                    ", "
                };
                united += country;
            }
            result +=
                &self.push_section("section_body", &serialize_string(&format!("from {united}")))?;
        }
        result += &self.push_section("section_title bold", "Winners Selection Date")?;
        result += &self.push_section(
            "section_body",
            &format_date_time(data.until_date, false, ".", ":", " ", dates)?,
        )?;
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
    pub fn push_giveaway_results(
        &mut self,
        peers: &PeersMap,
        data: &GiveawayResults,
        mut link: impl FnMut(i64, &str) -> String,
    ) -> Result<String> {
        let mut result =
            self.push_div("media_wrap clearfix", "") + &self.push_div("media_giveaway", "");
        let many = data.winners_count > 1;
        result += &self.push_section(
            "section_title bold",
            if many {
                "Winners Selected!"
            } else {
                "Winner Selected!"
            },
        )?;
        result += &self.push_section(
            "section_body",
            &format!(
                "<b>{}</b> {} of the {} was randomly selected by Telegram.",
                data.winners_count,
                if many { "winners" } else { "winner" },
                link(data.launch_id, "Giveaway")
            ),
        )?;
        result += &self.push_section(
            "section_title bold",
            if many { "Winners" } else { "Winner" },
        )?;
        let mut winners = data
            .winners
            .iter()
            .map(|id| format!("<b>{}</b>", peers.wrap_peer_name(*id)))
            .collect::<Vec<_>>()
            .join(", ");
        if data.winners_count > data.winners.len() as i64 {
            winners += &format!(
                " and {} more!",
                data.winners_count - data.winners.len() as i64
            );
        }
        result += &self.push_section("section_body", &winners)?;
        let stars = if data.credits == 1 { " Star" } else { " Stars" };
        let prize = if data.credits != 0 && data.winners_count == 1 {
            format!("The winner received <b>{}</b>{stars}.", data.credits)
        } else if data.credits != 0 && data.winners_count > 1 {
            format!(
                "All winners received <b>{}</b>{stars} in total.",
                data.credits
            )
        } else if data.unclaimed_count != 0 {
            serialize_string("Some winners couldn't be selected.")
        } else if data.winners_count == 1 {
            "The winner received their gift link in a private message.".into()
        } else if data.winners_count > 1 {
            "All winners received gift links in private messages.".into()
        } else {
            String::new()
        };
        result += &self.push_section("section_body", &prize)?;
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result)
    }
}
