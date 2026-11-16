//! Rich HTML structural blocks and complete message rendering.
pub use super::html_rich_text::{RichHtmlRenderer, RichMediaCallbacks};
use super::{
    error::Result,
    format::{DateContext, format_date_time, iso_date_time},
    html_rich_support::*,
    html_text::{HtmlContext, attrs, serialize_string},
    json_rich::enum_name,
    model_rich::*,
};
use std::collections::BTreeMap;
impl RichHtmlRenderer<'_> {
    pub fn render_message(&mut self) -> Result<String> {
        self.collect_anchors();
        let mut result = self.context.push_tag(
            "div",
            &attrs([
                ("class", "text rich_message"),
                ("data-rich-message", &self.message_id.to_string()),
                ("data-rich-part", rich_bool(self.message.part)),
                ("dir", if self.message.rtl { "rtl" } else { "auto" }),
                ("inline", ""),
            ]),
        );
        result += &self.render_blocks(&self.message.blocks)?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn text_block(
        &mut self,
        tag: &str,
        attributes: BTreeMap<String, String>,
        block: &RichBlock,
    ) -> Result<String> {
        let mut result = self.context.push_tag(tag, &attributes);
        result += &self.render_text(&block.text)?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn author_date(&mut self, block: &RichBlock) -> Result<String> {
        let mut a = rich_block_attributes("rich_author_date", "author-date");
        a.insert("dir".into(), "auto".into());
        let mut result = self.context.push_tag("address", &a);
        result += &self.render_text(&block.text)?;
        if block.date != 0 {
            result += " • ";
            result += &self.context.push_tag(
                "time",
                &attrs([
                    ("data-date", &block.date.to_string()),
                    ("datetime", &iso_date_time(block.date, self.dates)?),
                    ("dir", "ltr"),
                    ("inline", ""),
                ]),
            );
            result += &serialize_string(&format_date_time(
                block.date, false, ".", ":", " ", self.dates,
            )?);
            result += &self.context.pop_tag()?;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn code(&mut self, block: &RichBlock) -> Result<String> {
        let mut a = rich_block_attributes("rich_code", "code");
        a.insert("dir".into(), "auto".into());
        a.insert("inline".into(), String::new());
        if !block.language.is_empty() {
            a.insert("data-language".into(), block.language.clone());
        }
        let mut result = self.context.push_tag("pre", &a);
        result += &self.context.push_tag(
            "code",
            &attrs([("class", "rich_code_content"), ("inline", "")]),
        );
        result += &self.render_text(&block.text)?;
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn list(&mut self, block: &RichBlock) -> Result<String> {
        let ordered = block.list_kind == RichListKind::Ordered;
        let mut a =
            rich_block_attributes("rich_list", if ordered { "ordered-list" } else { "list" });
        if ordered {
            if block.ordered_list.reversed {
                a.insert("reversed".into(), String::new());
            }
            if let Some(start) = block.ordered_list.start {
                a.insert("start".into(), start.to_string());
            }
            if let Some(kind) = &block.ordered_list.r#type {
                a.insert(
                    if matches!(kind.as_str(), "1" | "a" | "A" | "i" | "I") {
                        "type"
                    } else {
                        "data-list-type"
                    }
                    .into(),
                    kind.clone(),
                );
            }
        }
        let mut result = self.context.push_tag(if ordered { "ol" } else { "ul" }, &a);
        for item in &block.list_items {
            result += &self.list_item(item, ordered)?;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn list_item(&mut self, item: &RichListItem, ordered: bool) -> Result<String> {
        let task = item.task_state != RichTaskState::None_;
        let checked = item.task_state == RichTaskState::Checked;
        let custom = ordered && !task && item.num.as_deref().is_some_and(|n| !n.is_empty());
        let class = format!(
            "rich_list_item{}",
            if task {
                " rich_task_item"
            } else if custom {
                " rich_order_item"
            } else {
                ""
            }
        );
        let blocks = item.content == RichListItemContent::Blocks;
        let mut a = attrs([
            ("class", &class),
            ("data-rich-content", if blocks { "blocks" } else { "text" }),
        ]);
        if let Some(num) = &item.num {
            a.insert("data-num".into(), num.clone());
        }
        if let Some(kind) = &item.r#type {
            a.insert("data-item-type".into(), kind.clone());
        }
        if let Some(value) = item.value {
            a.insert("value".into(), value.to_string());
        }
        let mut result = self.context.push_tag("li", &a);
        if task {
            result += &self.context.push_tag(
                "span",
                &attrs([
                    ("aria-checked", rich_bool(checked)),
                    ("class", "rich_task_marker"),
                    ("inline", ""),
                    ("role", "checkbox"),
                ]),
            );
            result += if checked { "☑" } else { "☐" };
            result += &self.context.pop_tag()?;
        } else if custom {
            result += &self.context.push_tag(
                "span",
                &attrs([("class", "rich_order_marker"), ("inline", "")]),
            );
            result += &serialize_string(item.num.as_deref().unwrap_or(""));
            result += &self.context.pop_tag()?;
        }
        result += &self.context.push_tag(
            if blocks { "div" } else { "span" },
            &attrs([("class", "rich_list_content"), ("inline", "")]),
        );
        if blocks {
            result += &self.render_blocks(&item.blocks)?;
        } else if let Some(text) = &item.text {
            result += &self.render_text(text)?;
        }
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn quote(&mut self, block: &RichBlock) -> Result<String> {
        let blocks = block.quote_content == RichQuoteContent::Blocks;
        let kind = if block.pullquote {
            "pullquote"
        } else if blocks {
            "quote-blocks"
        } else {
            "quote"
        };
        let classes = if block.pullquote {
            "rich_quote rich_pullquote"
        } else {
            "rich_quote"
        };
        let mut result = self.context.push_tag(
            if block.pullquote {
                "aside"
            } else {
                "blockquote"
            },
            &rich_block_attributes(classes, kind),
        );
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_quote_body")]));
        result += &if blocks {
            self.render_blocks(&block.blocks)?
        } else {
            self.render_text(&block.text)?
        };
        result += &self.context.pop_tag()?;
        let caption = self.render_text(&block.quote_caption)?;
        if !caption.is_empty() {
            result += &self
                .context
                .push_tag("cite", &attrs([("class", "rich_quote_caption")]));
            result += &caption;
            result += &self.context.pop_tag()?;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn details(&mut self, block: &RichBlock) -> Result<String> {
        let mut a = rich_block_attributes("rich_details", "details");
        if block.open {
            a.insert("open".into(), String::new());
        }
        let mut result = self.context.push_tag("details", &a);
        result += &self.context.push_tag("summary", &BTreeMap::new());
        result += &self.render_text(&block.text)?;
        result += &self.context.pop_tag()?;
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_details_body")]));
        result += &self.render_blocks(&block.blocks)?;
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn table(&mut self, block: &RichBlock) -> Result<String> {
        let mut a = rich_block_attributes("rich_table_wrap", "table");
        a.insert("tabindex".into(), "0".into());
        let mut result = self.context.push_tag("div", &a);
        let classes = format!(
            "rich_table{}{}{}",
            if block.bordered { " bordered" } else { "" },
            if block.striped { " striped" } else { "" },
            if block.compact { " compact" } else { "" }
        );
        result += &self
            .context
            .push_tag("table", &attrs([("class", &classes)]));
        let caption = self.render_text(&block.text)?;
        if !caption.is_empty() {
            result += &self.context.push_tag("caption", &BTreeMap::new());
            result += &caption;
            result += &self.context.pop_tag()?;
        }
        result += &self.context.push_tag("tbody", &BTreeMap::new());
        for row in &block.table_rows {
            result += &self.context.push_tag("tr", &BTreeMap::new());
            for cell in &row.cells {
                result += &self.table_cell(cell)?;
            }
            result += &self.context.pop_tag()?;
        }
        result += &self.context.pop_tag()?;
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn table_cell(&mut self, cell: &RichTableCell) -> Result<String> {
        let horizontal = match cell.alignment {
            RichTableAlignment::Left => "rich_align_left",
            RichTableAlignment::Center => "rich_align_center",
            RichTableAlignment::Right => "rich_align_right",
        };
        let vertical = match cell.vertical_alignment {
            RichTableVerticalAlignment::Top => "rich_valign_top",
            RichTableVerticalAlignment::Middle => "rich_valign_middle",
            RichTableVerticalAlignment::Bottom => "rich_valign_bottom",
        };
        let mut a = attrs([("class", &format!("{horizontal} {vertical}"))]);
        for (name, source) in [("colspan", cell.colspan), ("rowspan", cell.rowspan)] {
            if let Some(source) = source.filter(|s| *s > 0) {
                let span = source.min(1000);
                a.insert(name.into(), span.to_string());
                if span != source {
                    a.insert(format!("data-source-{name}"), source.to_string());
                }
            }
        }
        let mut result = self
            .context
            .push_tag(if cell.header { "th" } else { "td" }, &a);
        if let Some(text) = &cell.text {
            result += &self.render_text(text)?;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn button_row(&mut self, block: &RichBlock) -> Result<String> {
        let alignment = enum_name(block.button_alignment);
        let mut a = rich_block_attributes(
            &format!("rich_button_row rich_button_align_{alignment}"),
            "button-row",
        );
        a.insert("data-button-alignment".into(), alignment);
        let mut result = self.context.push_tag("div", &a);
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_button_group")]));
        for button in &block.buttons {
            result += &self.render_button(button, false)?;
        }
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn fallback(&mut self, kind: &str, label: &str) -> Result<String> {
        let result = self
            .context
            .push_tag("div", &rich_block_attributes("rich_fallback", kind));
        Ok(result + &serialize_string(label) + &self.context.pop_tag()?)
    }
    pub(crate) fn render_blocks(&mut self, blocks: &[RichBlock]) -> Result<String> {
        let mut result = String::new();
        for block in blocks {
            result += &self.render_block(block)?;
        }
        Ok(result)
    }
    pub fn render_block(&mut self, block: &RichBlock) -> Result<String> {
        use RichBlockKind as K;
        match block.kind {
            K::Unsupported => self.fallback("unsupported", "Unsupported rich content"),
            K::Heading => {
                let mut a = rich_block_attributes("rich_heading", "heading");
                a.insert("data-level".into(), block.heading_level.to_string());
                self.text_block(&rich_heading_tag(block.heading_level), a, block)
            }
            K::Paragraph | K::Footer | K::Thinking => {
                let name = match block.kind {
                    K::Paragraph => "paragraph",
                    K::Footer => "footer",
                    _ => "thinking",
                };
                let mut a = rich_block_attributes(&format!("rich_{name}"), name);
                a.insert("dir".into(), "auto".into());
                self.text_block(
                    if block.kind == K::Footer {
                        "footer"
                    } else {
                        "p"
                    },
                    a,
                    block,
                )
            }
            K::AuthorDate => self.author_date(block),
            K::Code => self.code(block),
            K::Divider => {
                let mut a = rich_block_attributes("rich_divider", "divider");
                a.insert("empty".into(), String::new());
                Ok(self.context.push_tag("hr", &a))
            }
            K::Anchor => {
                let Some(anchor) = self.block_anchor_id(block) else {
                    return self.fallback("malformed", "Malformed rich content");
                };
                let mut a = rich_block_attributes("rich_anchor", "anchor");
                a.insert("id".into(), anchor.into());
                a.insert("inline".into(), String::new());
                let result = self.context.push_tag("span", &a);
                Ok(result + &self.context.pop_tag()?)
            }
            K::Math => {
                let mut a = rich_block_attributes("rich_math_display", "math");
                a.insert("dir".into(), "ltr".into());
                a.insert("inline".into(), String::new());
                let result = self.context.push_tag("div", &a);
                Ok(result + &serialize_string(&block.formula) + &self.context.pop_tag()?)
            }
            K::List => self.list(block),
            K::Quote => self.quote(block),
            K::Photo => self.render_photo(block),
            K::Video => self.render_document(block, "video"),
            K::Audio => self.render_document(block, "audio"),
            K::File => self.render_document(block, "file"),
            K::Cover => self.render_media_group(block, "cover", false),
            K::Embed => self.render_embed(block),
            K::EmbedPost => self.render_embed_post(block),
            K::Collage => self.render_collage(block),
            K::Slideshow => self.render_slideshow(block),
            K::Channel => self.render_channel(block),
            K::Table => self.table(block),
            K::Details => self.details(block),
            K::RelatedArticles => self.render_related_articles(block),
            K::Map => self.render_map(block, "map"),
            K::InputMap => self.render_map(block, "input-map"),
            K::ButtonRow => self.button_row(block),
            K::Unknown => self.fallback("unknown", "Unknown rich content"),
        }
    }
}
pub fn render_rich_message(
    context: &mut HtmlContext,
    message: &RichMessage,
    message_id: i64,
    domain: &str,
    base: &str,
    media: &mut dyn RichMediaCallbacks,
    dates: &DateContext,
) -> Result<String> {
    let depth = context.tags.len();
    let result = RichHtmlRenderer::new(context, message, message_id, domain, base, media, dates)
        .render_message();
    if result.is_err() {
        context.tags.truncate(depth);
    }
    result
}
