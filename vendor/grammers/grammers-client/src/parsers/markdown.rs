// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![cfg(feature = "markdown")]

use grammers_tl_types as tl;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use super::common::{
    MENTION_URL_PREFIX, Segment, after, before, inject_into_message, telegram_string_len,
    update_entity_len,
};

/// Parse a message containing CommonMark-flavored markdown into plain text and the list of formatting entities understood by Telegram.
///
/// This is not the same as the markdown understood by Telegram's HTTP Bot API.
pub fn parse_markdown_message(message: &str) -> (String, Vec<tl::enums::MessageEntity>) {
    let mut text = String::with_capacity(message.len());
    let mut entities = Vec::new();

    let mut offset = 0;
    let length = 0;
    let mut pending_paragraph = false;
    for event in Parser::new(message) {
        if matches!(
            &event,
            Event::End(
                TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::Item
                    | TagEnd::List(_)
                    | TagEnd::BlockQuote(_)
            )
        ) {
            pending_paragraph = true;
            continue;
        }
        if pending_paragraph
            && matches!(
                &event,
                Event::Text(_)
                    | Event::Code(_)
                    | Event::HardBreak
                    | Event::SoftBreak
                    | Event::Start(
                        Tag::Strong | Tag::Emphasis | Tag::Link { .. } | Tag::CodeBlock(_)
                    )
            )
        {
            if !text.is_empty() {
                text.push_str("\n\n");
                offset += 2;
            }
            pending_paragraph = false;
        }
        match event {
            // text
            Event::Text(string) => {
                text.push_str(&string);
                offset += telegram_string_len(&string);
            }

            // `code`
            Event::Code(string) => {
                text.push_str(&string);
                let length = telegram_string_len(&string);
                entities.push(tl::types::MessageEntityCode { offset, length }.into());
                offset += length;
            }

            // **bold text**
            Event::Start(Tag::Strong) => {
                entities.push(tl::types::MessageEntityBold { offset, length }.into());
            }
            Event::End(TagEnd::Strong) => {
                update_entity_len!(Bold(offset) in entities);
            }

            // *italic text*
            Event::Start(Tag::Emphasis) => {
                entities.push(tl::types::MessageEntityItalic { offset, length }.into());
            }
            Event::End(TagEnd::Emphasis) => {
                update_entity_len!(Italic(offset) in entities);
            }

            // [text link](https://example.com) or [user mention](tg://user?id=12345678)
            Event::Start(Tag::Link { dest_url, .. }) => {
                if let Some(user_id) = dest_url
                    .strip_prefix(MENTION_URL_PREFIX)
                    .and_then(|id| id.parse::<i64>().ok())
                {
                    entities.push(
                        tl::types::MessageEntityMentionName {
                            offset,
                            length,
                            user_id,
                        }
                        .into(),
                    );
                } else {
                    entities.push(
                        tl::types::MessageEntityTextUrl {
                            offset,
                            length,
                            url: dest_url.to_string(),
                        }
                        .into(),
                    );
                }
            }
            Event::End(TagEnd::Link) => {
                let is_mention = entities
                    .iter()
                    .rev()
                    .find_map(|e| match e {
                        tl::enums::MessageEntity::MentionName(_) => Some(true),
                        tl::enums::MessageEntity::TextUrl(_) => Some(false),
                        _ => None,
                    })
                    .unwrap_or(false);

                if is_mention {
                    update_entity_len!(MentionName(offset) in entities);
                } else {
                    update_entity_len!(TextUrl(offset) in entities);
                }
            }

            // ```lang\npre```
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Indented => "".to_string(),
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                }
                .to_string();

                entities.push(
                    tl::types::MessageEntityPre {
                        offset,
                        length,
                        language: lang,
                    }
                    .into(),
                );
            }
            Event::End(TagEnd::CodeBlock) => {
                update_entity_len!(Pre(offset) in entities);
            }
            // "\\\n"
            Event::HardBreak | Event::SoftBreak => {
                text.push('\n');
                offset += 1;
            }
            Event::Rule => pending_paragraph = true,
            _ => {}
        }
    }

    (text, entities)
}

fn entity_text(message: &str, offset: i32, length: i32) -> String {
    String::from_utf16_lossy(
        &message
            .encode_utf16()
            .skip(offset.max(0) as usize)
            .take(length.max(0) as usize)
            .collect::<Vec<_>>(),
    )
}
fn longest_run(value: &str, delimiter: char) -> usize {
    value
        .chars()
        .scan(0, |run, c| {
            *run = if c == delimiter { *run + 1 } else { 0 };
            Some(*run)
        })
        .max()
        .unwrap_or(0)
}

fn emphasis_range(message: &str, offset: i32, length: i32) -> Option<(i32, i32)> {
    let content = entity_text(message, offset, length);
    let trimmed = content.trim_matches(char::is_whitespace);
    if trimmed.is_empty() {
        return None;
    }
    let leading = &content[..content.len() - content.trim_start_matches(char::is_whitespace).len()];
    let start = offset.checked_add(telegram_string_len(leading))?;
    Some((start, start.checked_add(telegram_string_len(trimmed))?))
}

fn merge_adjacent_code(entities: &[tl::enums::MessageEntity]) -> Vec<tl::enums::MessageEntity> {
    use tl::enums::MessageEntity as ME;
    let mut normalized = entities.to_vec();
    let mut indexes = normalized
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| match entity {
            ME::Code(code) if code.offset >= 0 && code.length > 0 => Some((code.offset, index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    indexes.sort_unstable();
    let mut removed = vec![false; normalized.len()];
    let mut previous = None;
    for (_, index) in indexes {
        let ME::Code(current) = &normalized[index] else {
            unreachable!()
        };
        let (offset, length) = (current.offset, current.length);
        if let Some(previous_index) = previous
            && let ME::Code(previous_code) = &mut normalized[previous_index]
            && previous_code.offset.checked_add(previous_code.length) == Some(offset)
            && let Some(combined_length) = previous_code.length.checked_add(length)
        {
            previous_code.length = combined_length;
            removed[index] = true;
            continue;
        }
        previous = Some(index);
    }
    normalized
        .into_iter()
        .enumerate()
        .filter_map(|(index, entity)| (!removed[index]).then_some(entity))
        .collect()
}

fn line_end_spaces(message: &str) -> std::collections::BTreeSet<usize> {
    let mut positions = std::collections::BTreeSet::new();
    let mut offset = message.encode_utf16().count();
    let mut at_end = true;
    for character in message.chars().rev() {
        offset -= character.len_utf16();
        match character {
            '\n' | '\r' => at_end = true,
            ' ' | '\t' if at_end => {
                positions.insert(offset);
            }
            _ => at_end = false,
        }
    }
    positions
}

/// Combine plain text and the list of formatting entities understood by Telegram into CommonMark-compliant markdown.
/// Adjacent inline-code ranges are merged. Emphasis excludes boundary whitespace,
/// which CommonMark cannot represent inside emphasis delimiters; the text is preserved.
pub fn generate_markdown_message(message: &str, entities: &[tl::enums::MessageEntity]) -> String {
    let entities = merge_adjacent_code(entities);
    // Getting this wrong isn't the end of the world so the wildcard pattern is used
    // (but it would still be a shame for it to be wrong).
    let mut insertions = Vec::with_capacity(
        entities
            .iter()
            .map(|entity| match entity {
                ME::Bold(_) => 2,
                ME::Italic(_) => 2,
                ME::Code(_) => 2,
                ME::Pre(e) => {
                    if e.language.is_empty() {
                        2
                    } else {
                        4
                    }
                }
                ME::TextUrl(_) => 4,
                ME::MentionName(_) => 4,
                _ => 0,
            })
            .sum(),
    );
    use tl::enums::MessageEntity as ME;
    entities
        .iter()
        .enumerate()
        .for_each(|(i, entity)| match entity {
            ME::Unknown(_) => {}
            ME::Mention(_) => {}
            ME::Hashtag(_) => {}
            ME::BotCommand(_) => {}
            ME::Url(_) => {}
            ME::Email(_) => {}
            ME::Bold(e) => {
                if let Some((start, end)) = emphasis_range(message, e.offset, e.length) {
                    insertions.push((before(i, 0, start), Segment::Fixed("**")));
                    insertions.push((after(i, 0, end), Segment::Fixed("**")));
                }
            }
            ME::Italic(e) => {
                let Some((start, end)) = emphasis_range(message, e.offset, e.length) else {
                    return;
                };
                let before_text = entity_text(message, 0, start);
                let after_text = entity_text(message, end, i32::MAX - end);
                let delimiter = if before_text
                    .chars()
                    .last()
                    .is_some_and(char::is_alphanumeric)
                    || after_text.chars().next().is_some_and(char::is_alphanumeric)
                {
                    "*"
                } else {
                    "_"
                };
                insertions.push((before(i, 0, start), Segment::Fixed(delimiter)));
                insertions.push((after(i, 0, end), Segment::Fixed(delimiter)));
            }
            ME::Code(e) => {
                let content = entity_text(message, e.offset, e.length);
                let mut offset = e.offset;
                // CommonMark normalizes line endings inside code spans to
                // spaces. Close each span before its line ending instead.
                for line in content.split_inclusive(['\r', '\n']) {
                    let segment = line.trim_end_matches(['\r', '\n']);
                    if !segment.is_empty() {
                        let delimiter = "`".repeat(longest_run(segment, '`') + 1);
                        let padding = segment.starts_with('`')
                            || segment.ends_with('`')
                            || segment.starts_with(' ')
                                && segment.ends_with(' ')
                                && segment.chars().any(|c| c != ' ');
                        let edge = format!("{delimiter}{}", if padding { " " } else { "" });
                        let end = format!("{}{delimiter}", if padding { " " } else { "" });
                        insertions.push((before(i, 0, offset), Segment::Owned(edge)));
                        insertions.push((
                            after(i, 0, offset + telegram_string_len(segment)),
                            Segment::Owned(end),
                        ));
                    }
                    offset += telegram_string_len(line);
                }
            }
            ME::Pre(e) => {
                let content = entity_text(message, e.offset, e.length);
                let delimiter = if e.language.contains('`') { '~' } else { '`' };
                let fence = delimiter
                    .to_string()
                    .repeat((longest_run(&content, delimiter) + 1).max(3));
                let language = e.language.split(['\r', '\n']).next().unwrap_or("");
                let before_text = entity_text(message, 0, e.offset);
                let boundary = if before_text.is_empty() || before_text.ends_with('\n') {
                    ""
                } else {
                    "\n\n"
                };
                insertions.push((
                    before(i, 0, e.offset),
                    Segment::Owned(format!("{boundary}{fence}{language}\n")),
                ));
                insertions.push((
                    after(i, 0, e.offset + e.length),
                    Segment::Owned(format!(
                        "{}{fence}\n",
                        if content.ends_with('\n') { "" } else { "\n" }
                    )),
                ));
            }
            ME::TextUrl(e) => {
                insertions.push((before(i, 0, e.offset), Segment::Fixed("[")));
                insertions.push((after(i, 0, e.offset + e.length), Segment::Fixed("](")));
                let mut destination = String::new();
                for c in e.url.chars() {
                    if matches!(c, '\\' | '(' | ')' | '<' | '>') {
                        destination.push('\\');
                    }
                    destination.push(c);
                }
                if e.url.chars().any(char::is_whitespace) {
                    destination = format!("<{destination}>");
                }
                insertions.push((
                    after(i, 1, e.offset + e.length),
                    Segment::Owned(destination),
                ));
                insertions.push((after(i, 2, e.offset + e.length), Segment::Fixed(")")));
            }
            ME::MentionName(e) => {
                insertions.push((before(i, 0, e.offset), Segment::Fixed("[")));
                insertions.push((
                    after(i, 0, e.offset + e.length),
                    Segment::Fixed("](tg://user?id="),
                ));
                insertions.push((after(i, 1, e.offset + e.length), Segment::Number(e.user_id)));
                insertions.push((after(i, 2, e.offset + e.length), Segment::Fixed(")")));
            }
            ME::InputMessageEntityMentionName(_) => {}
            ME::Phone(_) => {}
            ME::Cashtag(_) => {}
            ME::Underline(_) => {}
            ME::Strike(_) => {}
            ME::Blockquote(_) => {}
            ME::BankCard(_) => {}
            ME::Spoiler(_) => {}
            ME::CustomEmoji(_) => {}
            ME::FormattedDate(_) => {}
            ME::DiffInsert(_) => {}
            ME::DiffReplace(_) => {}
            ME::DiffDelete(_) => {}
            ME::TonAddress(_) => {}
        });

    let mut line_start = true;
    let line_end_spaces = line_end_spaces(message);
    inject_into_message(message, insertions, |result, c, position| {
        if matches!(c, '\r' | '\n')
            && entities.iter().any(|entity| {
                matches!(entity, ME::Code(e) if position as i64 >= i64::from(e.offset)
                && (position as i64) < i64::from(e.offset) + i64::from(e.length))
            })
        {
            // Numeric references keep leading/trailing and repeated line
            // endings as text, outside the separately delimited code spans.
            result.push_str(if c == '\r' { "&#13;" } else { "&#10;" });
            return;
        }
        let code = entities.iter().any(|entity| {
            let (offset, length) = match entity {
                ME::Code(e) => (e.offset, e.length),
                ME::Pre(e) => (e.offset, e.length),
                _ => return false,
            };
            let at = position as i64;
            at >= i64::from(offset) && at < i64::from(offset) + i64::from(length)
        });
        line_start |= result.ends_with('\n');
        if !code && (line_start || line_end_spaces.contains(&position)) && matches!(c, ' ' | '\t') {
            result.push_str(if c == ' ' { "&#32;" } else { "&#9;" });
            return;
        }
        line_start = c == '\n';
        if !code && c.is_ascii_punctuation() {
            result.push('\\');
        }
        result.push(c);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leading() {
        let (text, entities) = parse_markdown_message("**Hello** world!");
        assert_eq!(text, "Hello world!");
        assert_eq!(
            entities,
            vec![
                tl::types::MessageEntityBold {
                    offset: 0,
                    length: 5
                }
                .into()
            ]
        );
    }

    #[test]
    fn parse_trailing() {
        let (text, entities) = parse_markdown_message("Hello **world!**");
        assert_eq!(text, "Hello world!");
        assert_eq!(
            entities,
            vec![
                tl::types::MessageEntityBold {
                    offset: 6,
                    length: 6
                }
                .into()
            ]
        );
    }

    #[test]
    fn parse_emoji() {
        let (text, entities) = parse_markdown_message("A **little 🦀** here");
        assert_eq!(text, "A little 🦀 here");
        assert_eq!(
            entities,
            vec![
                tl::types::MessageEntityBold {
                    offset: 2,
                    length: 9
                }
                .into()
            ]
        );
    }

    #[test]
    fn parse_all_entities() {
        let (text, entities) = parse_markdown_message(
            "Some **bold** (__strong__), *italics* (_cursive_), inline `code`, \
            a\n```rust\npre\n```\nblock, a [**link**](https://example.com), and \
            [mentions](tg://user?id=12345678)",
        );

        assert_eq!(
            text,
            "Some bold (strong), italics (cursive), inline code, a\n\npre\nblock, a link, and mentions"
        );
        assert_eq!(
            entities,
            vec![
                tl::types::MessageEntityBold {
                    offset: 5,
                    length: 4
                }
                .into(),
                tl::types::MessageEntityBold {
                    offset: 11,
                    length: 6
                }
                .into(),
                tl::types::MessageEntityItalic {
                    offset: 20,
                    length: 7
                }
                .into(),
                tl::types::MessageEntityItalic {
                    offset: 29,
                    length: 7
                }
                .into(),
                tl::types::MessageEntityCode {
                    offset: 46,
                    length: 4
                }
                .into(),
                tl::types::MessageEntityPre {
                    offset: 55,
                    length: 4,
                    language: "rust".to_string()
                }
                .into(),
                tl::types::MessageEntityTextUrl {
                    offset: 68,
                    length: 4,
                    url: "https://example.com".to_string()
                }
                .into(),
                tl::types::MessageEntityBold {
                    offset: 68,
                    length: 4,
                }
                .into(),
                tl::types::MessageEntityMentionName {
                    offset: 78,
                    length: 8,
                    user_id: 12345678
                }
                .into(),
            ]
        );
    }

    #[test]
    fn parse_nested_entities() {
        // CommonMark won't allow the following: "Some **bold _both** italics_"
        let (text, entities) = parse_markdown_message("Some **bold _both_** _italics_");
        assert_eq!(text, "Some bold both italics");
        assert_eq!(
            entities,
            vec![
                tl::types::MessageEntityBold {
                    offset: 5,
                    length: 9
                }
                .into(),
                tl::types::MessageEntityItalic {
                    offset: 10,
                    length: 4
                }
                .into(),
                tl::types::MessageEntityItalic {
                    offset: 15,
                    length: 7
                }
                .into(),
            ]
        );
    }

    #[test]
    fn parse_then_unparse() {
        let markdown = "Some **bold 🤷🏽‍♀️**, _italics_, inline `🤷🏽‍♀️ code`, \
        a\n\n```rust\npre\n```\nblock, a [**link**](https://example.com), and \
        [mentions](tg://user?id=12345678)";
        let (text, entities) = parse_markdown_message(markdown);
        let generated = generate_markdown_message(&text, &entities);
        // Escaped punctuation is canonical output; its parsed message is unchanged.
        let expected = "Some **bold 🤷🏽‍♀️**\\, _italics_\\, inline `🤷🏽‍♀️ code`\\, \
        a\n\n```rust\npre\n```\nblock\\, a [**link**](https://example.com)\\, and \
        [mentions](tg://user?id=12345678)";
        assert_eq!(generated, expected);
        assert_eq!(parse_markdown_message(&generated), (text, entities));
    }

    #[test]
    fn parse_then_unparse_overlapping() {
        let markdown = "_a_[**b**](https://example.com)`c`";
        let (text, entities) = parse_markdown_message(markdown);
        let generated = generate_markdown_message(&text, &entities);
        assert_eq!(generated, "*a*[**b**](https://example.com)`c`");
        assert_eq!(parse_markdown_message(&generated), (text, entities));
    }
}
