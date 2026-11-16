//! Rich HTML anchors, inline text and nested-link state.
use super::{
    error::{ExportError, Result},
    format::{DateContext, format_date_time, iso_date_time},
    html_rich_support::*,
    html_safe::*,
    html_text::{HtmlContext, attrs, serialize_string},
    model::{Document, Photo, TextPart},
    model_rich::*,
};
use std::collections::BTreeMap;
pub trait RichMediaCallbacks {
    fn photo(&mut self, context: &mut HtmlContext, photo: Option<&Photo>) -> Result<String>;
    fn video(&mut self, context: &mut HtmlContext, document: Option<&Document>) -> Result<String>;
    fn audio(&mut self, context: &mut HtmlContext, document: Option<&Document>) -> Result<String>;
    fn file(&mut self, context: &mut HtmlContext, document: Option<&Document>) -> Result<String>;
    fn generic(&mut self, context: &mut HtmlContext, data: MediaData) -> Result<String>;
    fn photo_card(
        &mut self,
        context: &mut HtmlContext,
        data: MediaData,
        photo: Option<&Photo>,
    ) -> Result<String>;
}
pub struct RichHtmlRenderer<'a> {
    pub(crate) context: &'a mut HtmlContext,
    pub(crate) message: &'a RichMessage,
    pub(crate) message_id: i64,
    pub(crate) link_active: bool,
    pub(crate) domain: &'a str,
    pub(crate) base: &'a str,
    pub(crate) media: &'a mut dyn RichMediaCallbacks,
    pub(crate) dates: &'a DateContext,
    first_anchor_ids: BTreeMap<String, String>,
    anchor_occurrences: BTreeMap<String, u64>,
    text_anchor_ids: BTreeMap<usize, String>,
    block_anchor_ids: BTreeMap<usize, String>,
}
impl<'a> RichHtmlRenderer<'a> {
    pub fn new(
        context: &'a mut HtmlContext,
        message: &'a RichMessage,
        message_id: i64,
        domain: &'a str,
        base: &'a str,
        media: &'a mut dyn RichMediaCallbacks,
        dates: &'a DateContext,
    ) -> Self {
        Self {
            context,
            message,
            message_id,
            link_active: false,
            domain,
            base,
            media,
            dates,
            first_anchor_ids: BTreeMap::new(),
            anchor_occurrences: BTreeMap::new(),
            text_anchor_ids: BTreeMap::new(),
            block_anchor_ids: BTreeMap::new(),
        }
    }
    pub fn collect_anchors(&mut self) {
        self.first_anchor_ids.clear();
        self.anchor_occurrences.clear();
        self.text_anchor_ids.clear();
        self.block_anchor_ids.clear();
        self.collect_blocks(&self.message.blocks);
    }
    fn register_anchor(&mut self, name: &str) -> String {
        let base = format!(
            "rich-message-{}-anchor-{}",
            self.message_id,
            anchor_token(name)
        );
        let occurrence = self.anchor_occurrences.entry(base.clone()).or_default();
        *occurrence += 1;
        let result = if *occurrence > 1 {
            format!("{base}-{occurrence}")
        } else {
            base
        };
        self.first_anchor_ids
            .entry(name.into())
            .or_insert_with(|| result.clone());
        result
    }
    fn collect_text(&mut self, text: &RichText) {
        use RichTextType as T;
        match text.r#type {
            T::Anchor => {
                let id = self.register_anchor(&text.data);
                self.text_anchor_ids
                    .entry(std::ptr::from_ref(text) as usize)
                    .or_insert(id);
                for child in &text.children {
                    self.collect_text(child);
                }
            }
            T::Diff => {
                for child in text.children.iter().chain(&text.old_children) {
                    self.collect_text(child);
                }
            }
            T::Empty | T::Plain | T::Math | T::CustomEmoji | T::InlineImage => {}
            _ => {
                for child in &text.children {
                    self.collect_text(child);
                }
            }
        }
    }
    fn collect_caption(&mut self, caption: &RichCaption) {
        self.collect_text(&caption.text);
        self.collect_text(&caption.credit);
    }
    fn collect_blocks(&mut self, blocks: &[RichBlock]) {
        for block in blocks {
            use RichBlockKind as K;
            match block.kind {
                K::Heading
                | K::Paragraph
                | K::Footer
                | K::Thinking
                | K::AuthorDate
                | K::Code
                | K::RelatedArticles => self.collect_text(&block.text),
                K::Anchor => {
                    let id = self.register_anchor(&block.name);
                    self.block_anchor_ids
                        .entry(std::ptr::from_ref(block) as usize)
                        .or_insert(id);
                }
                K::List => {
                    for item in &block.list_items {
                        if item.content == RichListItemContent::Text {
                            if let Some(text) = &item.text {
                                self.collect_text(text);
                            }
                        } else {
                            self.collect_blocks(&item.blocks);
                        }
                    }
                }
                K::Quote => {
                    if block.quote_content == RichQuoteContent::Text {
                        self.collect_text(&block.text);
                    } else {
                        self.collect_blocks(&block.blocks);
                    }
                    self.collect_text(&block.quote_caption);
                }
                K::Photo | K::Video | K::Audio | K::File | K::Embed | K::Map | K::InputMap => {
                    self.collect_caption(&block.caption)
                }
                K::EmbedPost | K::Collage | K::Slideshow => {
                    self.collect_blocks(&block.blocks);
                    self.collect_caption(&block.caption);
                }
                K::Cover => self.collect_blocks(&block.blocks),
                K::Table => {
                    self.collect_text(&block.text);
                    for row in &block.table_rows {
                        for cell in &row.cells {
                            if let Some(text) = &cell.text {
                                self.collect_text(text);
                            }
                        }
                    }
                }
                K::Details => {
                    self.collect_text(&block.text);
                    self.collect_blocks(&block.blocks);
                }
                K::ButtonRow => {
                    for button in &block.buttons {
                        self.collect_text(button);
                    }
                }
                _ => {}
            }
        }
    }
    fn fragment_href(&self, target: &str) -> Option<String> {
        if !is_strict_target(target) || !target.starts_with('#') {
            return None;
        }
        self.first_anchor_ids
            .get(&target[1..])
            .map(|id| format!("#{id}"))
    }
    pub(crate) fn block_anchor_id(&self, block: &RichBlock) -> Option<&str> {
        self.block_anchor_ids
            .get(&(std::ptr::from_ref(block) as usize))
            .map(String::as_str)
    }
    pub fn render_texts(&mut self, texts: &[RichText]) -> Result<String> {
        let mut result = String::new();
        for text in texts {
            result += &self.render_text(text)?;
        }
        Ok(result)
    }
    fn unsupported_text(&mut self) -> Result<String> {
        let result = self.context.push_tag(
            "span",
            &attrs([
                ("class", "rich_inline_fallback"),
                ("data-rich-kind", "unsupported-text"),
                ("inline", ""),
            ]),
        );
        Ok(result + "Unsupported rich text" + &self.context.pop_tag()?)
    }
    fn render_children(&mut self, text: &RichText) -> Result<String> {
        if text.children.is_empty() {
            self.unsupported_text()
        } else {
            self.render_texts(&text.children)
        }
    }
    fn render_link(
        &mut self,
        text: &RichText,
        href: Option<String>,
        mut attributes: BTreeMap<String, String>,
    ) -> Result<String> {
        attributes.entry("inline".into()).or_default();
        if href.is_none() || self.link_active {
            attributes.remove("href");
            attributes.remove("onclick");
            attributes
                .entry("class".into())
                .or_insert_with(|| "rich_inert_link".into());
            let mut result = self.context.push_tag("span", &attributes);
            result += &self.render_children(text)?;
            return Ok(result + &self.context.pop_tag()?);
        }
        attributes
            .entry("href".into())
            .or_insert(href.unwrap_or_default());
        let mut result = self.context.push_tag("a", &attributes);
        let previous = self.link_active;
        self.link_active = true;
        let child = self.render_children(text);
        self.link_active = previous;
        result += &child?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub fn render_button(&mut self, button: &RichText, inline: bool) -> Result<String> {
        let payload = button.button.as_ref().ok_or(ExportError::InvalidRecord)?;
        if button.r#type != RichTextType::Button || button.children.len() != 1 {
            return Err(ExportError::InvalidRecord);
        }
        let action = &payload.action;
        let mut attributes = rich_button_attributes(action, payload.style, inline);
        let mut tag = "span";
        let url = if action.r#type == InlineButtonActionType::Url {
            safe_message_href(&action.url)
        } else {
            None
        };
        if !self.link_active && url.is_some() {
            tag = "a";
            attributes.insert("href".into(), url.unwrap_or_default());
        } else if !self.link_active && action.r#type == InlineButtonActionType::CopyText {
            tag = "button";
            attributes.insert("type".into(), "button".into());
            attributes.insert(
                "onclick".into(),
                "return ShowTextCopied(decodeURIComponent(this.dataset.copyText))".into(),
            );
        }
        let mut result = self.context.push_tag(tag, &attributes);
        let previous = self.link_active;
        self.link_active = true;
        let inner = self.render_text(&button.children[0]);
        self.link_active = previous;
        result += &inner?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn custom_emoji_link(
        &mut self,
        alt: &str,
        mut attributes: BTreeMap<String, String>,
    ) -> Result<String> {
        attributes.entry("inline".into()).or_default();
        if self.link_active {
            attributes.remove("href");
            attributes.remove("onclick");
            attributes
                .entry("class".into())
                .or_insert_with(|| "rich_inert_link".into());
            let result = self.context.push_tag("span", &attributes);
            return Ok(result + alt + &self.context.pop_tag()?);
        }
        let result = self.context.push_tag("a", &attributes);
        Ok(result + alt + &self.context.pop_tag()?)
    }
    fn wrap_children(
        &mut self,
        tag: &str,
        mut attributes: BTreeMap<String, String>,
        text: &RichText,
    ) -> Result<String> {
        attributes.entry("inline".into()).or_default();
        let mut result = self.context.push_tag(tag, &attributes);
        result += &self.render_children(text)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub fn render_text(&mut self, text: &RichText) -> Result<String> {
        use RichTextType as T;
        match text.r#type {
            T::Empty => Ok(String::new()),
            T::Plain => Ok(serialize_string(&text.text)),
            T::Concat => self.render_texts(&text.children),
            T::Bold
            | T::Italic
            | T::Underline
            | T::Strike
            | T::Fixed
            | T::Subscript
            | T::Superscript
            | T::Marked
            | T::BankCard => {
                let (tag, class) = match text.r#type {
                    T::Bold => ("strong", None),
                    T::Italic => ("em", None),
                    T::Underline => ("u", None),
                    T::Strike => ("s", None),
                    T::Fixed => ("code", Some("rich_inline_code")),
                    T::Subscript => ("sub", None),
                    T::Superscript => ("sup", None),
                    T::Marked => ("mark", None),
                    _ => ("span", Some("rich_bank_card")),
                };
                let mut attributes = BTreeMap::new();
                if let Some(class) = class {
                    attributes.insert("class".into(), class.into());
                }
                self.wrap_children(tag, attributes, text)
            }
            T::Url => {
                let href = if text.data.starts_with('#') {
                    self.fragment_href(&text.data)
                } else {
                    safe_http_href(&text.data)
                };
                let mut extra = BTreeMap::new();
                if text.id != 0 {
                    extra.insert("data-webpage-id".into(), text.id.to_string());
                }
                self.render_link(text, href, extra)
            }
            T::Email => self.render_link(text, safe_email_href(&text.data), BTreeMap::new()),
            T::Phone => self.render_link(text, safe_phone_href(&text.data), BTreeMap::new()),
            T::Anchor => {
                let mut attributes = attrs([("class", "rich_reference"), ("inline", "")]);
                if let Some(anchor) = self
                    .text_anchor_ids
                    .get(&(std::ptr::from_ref(text) as usize))
                {
                    attributes.insert("id".into(), anchor.clone());
                }
                let mut result = self.context.push_tag("span", &attributes);
                result += &self.render_texts(&text.children)?;
                Ok(result + &self.context.pop_tag()?)
            }
            T::Math => {
                let result = self.context.push_tag(
                    "span",
                    &attrs([
                        ("class", "rich_math_inline"),
                        ("data-rich-kind", "inline-math"),
                        ("dir", "ltr"),
                        ("inline", ""),
                    ]),
                );
                Ok(result + &serialize_string(&text.data) + &self.context.pop_tag()?)
            }
            T::CustomEmoji => self.render_custom_emoji(text),
            T::Spoiler => {
                let mut result = self.context.push_tag(
                    "span",
                    &attrs([
                        ("class", "spoiler hidden"),
                        ("inline", ""),
                        ("onclick", "ShowSpoiler(this)"),
                    ]),
                );
                result += &self
                    .context
                    .push_tag("span", &attrs([("aria-hidden", "true"), ("inline", "")]));
                result += &self.render_children(text)?;
                result += &self.context.pop_tag()?;
                Ok(result + &self.context.pop_tag()?)
            }
            T::Mention => {
                let href = plain_target(text).and_then(|t| safe_mention_href(&t, self.domain));
                self.render_link(text, href, BTreeMap::new())
            }
            T::Hashtag | T::BotCommand | T::Cashtag => self.render_action(text),
            T::AutoUrl | T::AutoEmail | T::AutoPhone => {
                let href = plain_target(text).and_then(|t| match text.r#type {
                    T::AutoUrl => safe_http_href(&t),
                    T::AutoEmail => safe_email_href(&t),
                    _ => safe_phone_href(&t),
                });
                self.render_link(text, href, BTreeMap::new())
            }
            T::MentionName => self.render_link(
                text,
                Some(String::new()),
                attrs([
                    ("data-user-id", &text.id.to_string()),
                    ("onclick", "return ShowMentionName()"),
                ]),
            ),
            T::FormattedDate => self.render_formatted_date(text),
            T::InlineImage => {
                let result = self.context.push_tag(
                    "span",
                    &attrs([
                        ("class", "rich_inline_fallback"),
                        ("data-document-id", &text.id.to_string()),
                        ("data-rich-kind", "inline-image"),
                        ("data-source-height", &text.height.to_string()),
                        ("data-source-width", &text.width.to_string()),
                        ("inline", ""),
                    ]),
                );
                Ok(result + "Inline image" + &self.context.pop_tag()?)
            }
            T::Diff => {
                let mut result = self.context.push_tag(
                    "span",
                    &attrs([
                        ("class", "rich_diff"),
                        ("data-rich-kind", "diff"),
                        ("inline", ""),
                    ]),
                );
                result += &self.render_children(text)?;
                result += &self.context.push_tag(
                    "del",
                    &attrs([("class", "rich_diff_previous"), ("inline", "")]),
                );
                result += "Previous: ";
                result += &if text.old_children.is_empty() {
                    self.unsupported_text()?
                } else {
                    self.render_texts(&text.old_children)?
                };
                result += &self.context.pop_tag()?;
                Ok(result + &self.context.pop_tag()?)
            }
            T::Button => self.render_button(text, true),
        }
    }
    fn render_custom_emoji(&mut self, text: &RichText) -> Result<String> {
        let mut result = self.context.push_tag(
            "span",
            &attrs([
                ("class", "rich_custom_emoji"),
                ("data-document-id", &text.id.to_string()),
                ("inline", ""),
            ]),
        );
        let alt = serialize_string(&text.text);
        let unavailable = text.custom_emoji_data == TextPart::UNAVAILABLE_EMOJI;
        let unloaded =
            text.custom_emoji_data.is_empty() || is_ascii_decimal(&text.custom_emoji_data);
        let href = if unavailable || unloaded {
            None
        } else {
            safe_relative_emoji_href(&text.custom_emoji_data, self.base)
        };
        if let Some(href) = href {
            result += &self.custom_emoji_link(&alt, attrs([("href", &href)]))?;
        } else if unavailable || unloaded {
            result += &self.custom_emoji_link(
                &alt,
                attrs([
                    ("href", ""),
                    (
                        "onclick",
                        if unavailable {
                            "return ShowNotAvailableEmoji()"
                        } else {
                            "return ShowNotLoadedEmoji()"
                        },
                    ),
                ]),
            )?;
        } else {
            result += &alt;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn render_action(&mut self, text: &RichText) -> Result<String> {
        let (prefix, attribute, callback) = match text.r#type {
            RichTextType::Hashtag => ("#", "data-tag", "return ShowHashtag(this.dataset.tag)"),
            RichTextType::BotCommand => (
                "/",
                "data-command",
                "return ShowBotCommand(this.dataset.command)",
            ),
            _ => ("$", "data-tag", "return ShowCashtag(this.dataset.tag)"),
        };
        if let Some(action) = plain_target(text).and_then(|t| safe_action_data(&t, prefix)) {
            self.render_link(
                text,
                Some(String::new()),
                attrs([(attribute, &action), ("onclick", callback)]),
            )
        } else {
            self.render_link(text, None, BTreeMap::new())
        }
    }
    fn render_formatted_date(&mut self, text: &RichText) -> Result<String> {
        let mut attributes = attrs([
            ("class", "rich_formatted_date"),
            ("data-date", &text.date.to_string()),
            ("datetime", &iso_date_time(text.date, self.dates)?),
            ("dir", "ltr"),
            ("inline", ""),
            (
                "title",
                &format_date_time(text.date, false, ".", ":", " ", self.dates)?,
            ),
        ]);
        for (k, v) in [
            ("data-day-of-week", text.day_of_week),
            ("data-long-date", text.long_date),
            ("data-long-time", text.long_time),
            ("data-relative", text.relative),
            ("data-short-date", text.short_date),
            ("data-short-time", text.short_time),
        ] {
            attributes.insert(k.into(), rich_bool(v).into());
        }
        let mut result = self.context.push_tag("time", &attributes);
        result += &self.render_children(text)?;
        Ok(result + &self.context.pop_tag()?)
    }
}
