//! Rich HTML media, groups, references, geometry and tail-state rules.
use super::{
    error::Result,
    format::format_date_time,
    html_layout::{layout_media_group_geometry, united},
    html_rich_support::*,
    html_rich_text::{RichHtmlRenderer, RichMediaCallbacks},
    html_safe::{is_ascii_word, safe_http_href},
    html_text::{HtmlContext, WrapBase, attrs, serialize_string},
    model::{Document, Photo},
    model_rich::*,
};
use std::{collections::BTreeMap, path::PathBuf};
pub struct NativeRichMedia {
    pub base: String,
    pub folder: PathBuf,
}
impl NativeRichMedia {
    pub fn new(base: impl Into<String>, folder: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            folder: folder.into(),
        }
    }
    fn with_wrap(
        &self,
        context: &mut HtmlContext,
        operation: impl FnOnce(&mut WrapBase) -> Result<String>,
    ) -> Result<String> {
        let mut wrap = WrapBase {
            base: self.base.clone(),
            context: std::mem::take(context),
        };
        let output = operation(&mut wrap);
        *context = wrap.context;
        output
    }
}
impl RichMediaCallbacks for NativeRichMedia {
    fn photo(&mut self, c: &mut HtmlContext, p: Option<&Photo>) -> Result<String> {
        self.with_wrap(c, |w| w.push_rich_photo_media(p, &self.folder))
    }
    fn video(&mut self, c: &mut HtmlContext, d: Option<&Document>) -> Result<String> {
        self.with_wrap(c, |w| w.push_rich_video_media(d, &self.folder))
    }
    fn audio(&mut self, c: &mut HtmlContext, d: Option<&Document>) -> Result<String> {
        self.with_wrap(c, |w| w.push_rich_audio_media(d))
    }
    fn file(&mut self, c: &mut HtmlContext, d: Option<&Document>) -> Result<String> {
        self.with_wrap(c, |w| w.push_rich_file_media(d))
    }
    fn generic(&mut self, c: &mut HtmlContext, d: MediaData) -> Result<String> {
        self.with_wrap(c, |w| w.push_generic_media(&d))
    }
    fn photo_card(
        &mut self,
        c: &mut HtmlContext,
        mut d: MediaData,
        p: Option<&Photo>,
    ) -> Result<String> {
        self.with_wrap(c, |w| w.push_rich_reference_media(&mut d, p, &self.folder))
    }
}
pub use super::html_rich_state::RichTailState;
impl RichHtmlRenderer<'_> {
    pub(crate) fn render_caption(&mut self, caption: &RichCaption) -> Result<String> {
        let text = self.render_text(&caption.text)?;
        let credit = self.render_text(&caption.credit)?;
        if text.is_empty() && credit.is_empty() {
            return Ok(String::new());
        }
        let mut result = self.context.push_tag(
            "figcaption",
            &attrs([("class", "rich_media_caption"), ("dir", "auto")]),
        );
        if !text.is_empty() {
            result += &self
                .context
                .push_tag("div", &attrs([("class", "rich_caption_text")]));
            result += &text;
            result += &self.context.pop_tag()?;
        }
        if !credit.is_empty() {
            result += &self
                .context
                .push_tag("cite", &attrs([("class", "rich_caption_credit")]));
            result += &credit;
            result += &self.context.pop_tag()?;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn render_source_meta(&mut self, source: &str, embed_post: bool) -> Result<String> {
        if source.is_empty() {
            return Ok(String::new());
        }
        let mut result = self.context.push_tag(
            "div",
            &attrs([(
                "class",
                if embed_post {
                    "rich_embed_post_meta"
                } else {
                    "rich_source_meta"
                },
            )]),
        );
        if let Some(href) = safe_http_href(source).filter(|_| !self.link_active) {
            result += &self.context.push_tag(
                "a",
                &attrs([
                    ("class", "rich_source_link"),
                    ("href", &href),
                    ("inline", ""),
                ]),
            );
        } else {
            result += &self.context.push_tag(
                "span",
                &attrs([("class", "rich_source_text"), ("inline", "")]),
            );
        }
        result += &serialize_string(source);
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_photo(&mut self, block: &RichBlock) -> Result<String> {
        let mut a = rich_block_attributes("rich_media_item", "photo");
        a.insert("data-photo-id".into(), block.photo_id.to_string());
        a.insert("data-spoiler".into(), rich_bool(block.spoiler).into());
        if let Some(id) = block.optional_webpage_id {
            a.insert("data-webpage-id".into(), id.to_string());
        }
        let mut result = self.context.push_tag("figure", &a);
        result += &self.media.photo(
            self.context,
            self.message
                .photos
                .get(&block.photo_id)
                .filter(|_| block.photo_id != 0),
        )?;
        if let Some(url) = &block.optional_url {
            result += &self.render_source_meta(url, false)?;
        }
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_document(&mut self, block: &RichBlock, kind: &str) -> Result<String> {
        let mut a = rich_block_attributes("rich_media_item", kind);
        a.insert("data-document-id".into(), block.document_id.to_string());
        if kind == "video" {
            a.insert("data-autoplay".into(), rich_bool(block.autoplay).into());
            a.insert("data-loop".into(), rich_bool(block.r#loop).into());
            a.insert("data-spoiler".into(), rich_bool(block.spoiler).into());
        }
        let mut result = self.context.push_tag("figure", &a);
        let document = self
            .message
            .documents
            .get(&block.document_id)
            .filter(|_| block.document_id != 0);
        result += &match kind {
            "video" => self.media.video(self.context, document)?,
            "audio" => self.media.audio(self.context, document)?,
            _ => self.media.file(self.context, document)?,
        };
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_media_group(
        &mut self,
        block: &RichBlock,
        kind: &str,
        caption: bool,
    ) -> Result<String> {
        let mut a = rich_block_attributes("rich_media_group", kind);
        a.insert(
            "data-rich-has-items".into(),
            rich_bool(!block.blocks.is_empty()).into(),
        );
        a.insert(
            "data-rich-items-end-media".into(),
            rich_bool(self.tail_state_of(&block.blocks)? == RichTailState::Media).into(),
        );
        let mut result = self.context.push_tag("section", &a);
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_media_group_items")]));
        result += &self.render_blocks(&block.blocks)?;
        result += &self.context.pop_tag()?;
        if caption {
            result += &self.render_caption(&block.caption)?;
        }
        Ok(result + &self.context.pop_tag()?)
    }
    fn full_group_attributes(kind: &str) -> BTreeMap<String, String> {
        let mut a = rich_block_attributes("rich_media_group", kind);
        a.insert("data-rich-has-items".into(), "true".into());
        a.insert("data-rich-items-end-media".into(), "true".into());
        a
    }
    pub(crate) fn render_collage(&mut self, block: &RichBlock) -> Result<String> {
        let sizes = self.media_group_sizes(&block.blocks);
        if sizes.is_empty() {
            return self.render_media_group(block, "collage", true);
        }
        let layout = layout_media_group_geometry(&sizes, 430, 100, 4)?;
        let mut full = None;
        for rect in &layout {
            full = Some(united(full, *rect));
        }
        let Some((fx, fy, fw, fh)) = full.filter(|r| r.2 > 0 && r.3 > 0) else {
            return self.render_media_group(block, "collage", true);
        };
        if layout.len() != sizes.len() {
            return self.render_media_group(block, "collage", true);
        }
        let mut result = self
            .context
            .push_tag("section", &Self::full_group_attributes("collage"));
        result += &self.context.push_tag(
            "div",
            &attrs([
                ("class", "rich_collage_box"),
                ("style", &rich_aspect_ratio_style(fw, fh)),
            ]),
        );
        for ((x, y, w, h), item) in layout.into_iter().zip(&block.blocks) {
            let style = format!(
                "left: {}; top: {}; width: {}; height: {}",
                rich_percent_value(x - fx, fw),
                rich_percent_value(y - fy, fh),
                rich_percent_value(w, fw),
                rich_percent_value(h, fh)
            );
            result += &self.context.push_tag(
                "div",
                &attrs([("class", "rich_collage_item"), ("style", &style)]),
            );
            result += &self.render_block(item)?;
            result += &self.context.pop_tag()?;
        }
        result += &self.context.pop_tag()?;
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_slideshow(&mut self, block: &RichBlock) -> Result<String> {
        let sizes = self.media_group_sizes(&block.blocks);
        let Some(mut widest) = sizes.first().copied() else {
            return self.render_media_group(block, "slideshow", true);
        };
        for size in sizes {
            if size.0 * widest.1 > widest.0 * size.1 {
                widest = size;
            }
        }
        let count = block.blocks.len();
        let mut result = self
            .context
            .push_tag("section", &Self::full_group_attributes("slideshow"));
        result += &self.context.push_tag(
            "div",
            &attrs([
                ("class", "rich_slideshow_box"),
                ("style", &rich_aspect_ratio_style(widest.0, widest.1)),
            ]),
        );
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_slideshow_track")]));
        for item in &block.blocks {
            result += &self
                .context
                .push_tag("div", &attrs([("class", "rich_slide")]));
            result += &self.render_block(item)?;
            result += &self.context.pop_tag()?;
        }
        result += &self.context.pop_tag()?;
        if count > 1 {
            for (name, arrow) in [("prev", "&#8592;"), ("next", "&#8594;")] {
                result += &self.context.push_tag(
                    "button",
                    &attrs([
                        ("class", &format!("rich_slideshow_{name}")),
                        ("type", "button"),
                        ("inline", ""),
                    ]),
                );
                result += arrow;
                result += &self.context.pop_tag()?;
            }
            result += &self
                .context
                .push_tag("div", &attrs([("class", "rich_slideshow_dots")]));
            for index in 0..count {
                result += &self.context.push_tag(
                    "button",
                    &attrs([
                        ("aria-label", &format!("Show slide {}", index + 1)),
                        ("class", "rich_slideshow_dot"),
                        ("type", "button"),
                        ("inline", ""),
                    ]),
                );
                result += &self.context.pop_tag()?;
            }
            result += &self.context.pop_tag()?;
        }
        result += &self.context.pop_tag()?;
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_embed(&mut self, block: &RichBlock) -> Result<String> {
        let poster = block.poster_photo_id;
        let mut a = rich_block_attributes("rich_media_item rich_embed", "embed");
        a.insert("data-full-width".into(), rich_bool(block.full_width).into());
        a.insert(
            "data-allow-scrolling".into(),
            rich_bool(block.allow_scrolling).into(),
        );
        a.insert(
            "data-rich-has-meta".into(),
            rich_bool(
                poster.is_some() && block.optional_url.as_deref().is_some_and(|u| !u.is_empty()),
            )
            .into(),
        );
        for (key, value) in [
            ("data-poster-photo-id", poster),
            ("data-source-width", block.width),
            ("data-source-height", block.height),
        ] {
            if let Some(value) = value {
                a.insert(key.into(), value.to_string());
            }
        }
        let mut result = self.context.push_tag("figure", &a);
        if let Some(id) = poster {
            result += &self.media.photo(
                self.context,
                self.message.photos.get(&id).filter(|_| id != 0),
            )?;
            if let Some(url) = &block.optional_url {
                result += &self.render_source_meta(url, false)?;
            }
        } else {
            let mut media = MediaData {
                title: "Embed".into(),
                classes: "media_file".into(),
                ..Default::default()
            };
            if let Some(url) = &block.optional_url {
                media.status = url.clone();
                if let Some(href) = safe_http_href(url) {
                    media.link = href;
                }
            }
            result += &self.media.generic(self.context, media)?;
        }
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_embed_post(&mut self, block: &RichBlock) -> Result<String> {
        let photo = self
            .message
            .photos
            .get(&block.author_photo_id)
            .filter(|_| block.author_photo_id != 0);
        let mut a = rich_block_attributes("rich_embed_post", "embed-post");
        for (key, value) in [
            ("data-webpage-id", block.webpage_id),
            ("data-author-photo-id", block.author_photo_id),
            ("data-date", block.date),
        ] {
            a.insert(key.into(), value.to_string());
        }
        a.insert(
            "data-rich-has-meta".into(),
            rich_bool(!block.url.is_empty()).into(),
        );
        a.insert(
            "data-rich-has-items".into(),
            rich_bool(!block.blocks.is_empty()).into(),
        );
        a.insert(
            "data-rich-items-end-media".into(),
            rich_bool(self.tail_state_of(&block.blocks)? == RichTailState::Media).into(),
        );
        let mut result = self.context.push_tag("section", &a);
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_embed_post_author")]));
        let media = MediaData {
            title: if block.author.is_empty() {
                "Embed post".into()
            } else {
                block.author.clone()
            },
            description: rich_file_description(photo.map(|p| &p.image.file))?.into(),
            status: if block.date != 0 {
                format_date_time(block.date, false, ".", ":", " ", self.dates)?
            } else {
                String::new()
            },
            classes: "media_contact".into(),
            ..Default::default()
        };
        result += &self.media.photo_card(self.context, media, photo)?;
        result += &self.context.pop_tag()?;
        result += &self.render_source_meta(&block.url, true)?;
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_embed_post_blocks")]));
        result += &self.render_blocks(&block.blocks)?;
        result += &self.context.pop_tag()?;
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_channel(&mut self, block: &RichBlock) -> Result<String> {
        let c = &block.channel;
        let mut a = rich_block_attributes("rich_reference_block", "channel");
        a.insert(
            "data-channel-source".into(),
            rich_channel_source_name(c.source),
        );
        a.insert("data-channel-id".into(), c.id.to_string());
        a.insert("data-broadcast".into(), rich_bool(c.broadcast).into());
        a.insert("data-megagroup".into(), rich_bool(c.megagroup).into());
        a.insert("data-monoforum".into(), rich_bool(c.monoforum).into());
        if let Some(hash) = c.access_hash {
            a.insert("data-access-hash".into(), hash.to_string());
        }
        let mut result = self.context.push_tag("section", &a);
        let mut media = MediaData {
            title: c
                .title
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("Channel")
                .into(),
            status: rich_channel_status(c),
            classes: "media_contact".into(),
            ..Default::default()
        };
        if let Some(username) = c.username.as_deref().filter(|u| is_ascii_word(u, 5, 32))
            && let Some(href) = safe_http_href(&format!("{}{username}", self.domain))
        {
            media.link = href;
        }
        result += &self.media.generic(self.context, media)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_map(&mut self, block: &RichBlock, kind: &str) -> Result<String> {
        let p = &block.map_point;
        let mut a = rich_block_attributes("rich_reference_block", kind);
        a.insert(
            "data-point-source".into(),
            rich_map_point_source_name(p.source),
        );
        for (key, value) in [
            ("data-zoom", block.zoom),
            ("data-source-width", block.map_width),
            ("data-source-height", block.map_height),
        ] {
            a.insert(key.into(), value.to_string());
        }
        for (key, value) in [
            ("data-access-hash", p.access_hash),
            ("data-accuracy-radius", p.accuracy_radius),
        ] {
            if let Some(value) = value {
                a.insert(key.into(), value.to_string());
            }
        }
        let mut result = self.context.push_tag("figure", &a);
        let mut media = MediaData {
            title: "Location".into(),
            classes: "media_location".into(),
            ..Default::default()
        };
        if let Some((lat, lon)) = rich_map_coordinates(p) {
            media.status = format!("{lat}, {lon}");
            if let Some(href) = safe_http_href(&format!(
                "https://maps.google.com/maps?q={lat},{lon}&ll={lat},{lon}&z=16"
            )) {
                media.link = href;
            }
        }
        result += &self.media.generic(self.context, media)?;
        result += &self.render_caption(&block.caption)?;
        Ok(result + &self.context.pop_tag()?)
    }
    fn render_related_article(
        &mut self,
        article: &RichRelatedArticle,
        index: usize,
    ) -> Result<String> {
        let mut a = attrs([
            ("class", "rich_related_article"),
            ("data-rich-article-index", &index.to_string()),
            ("data-webpage-id", &article.webpage_id.to_string()),
        ]);
        for (key, value) in [
            ("data-photo-id", article.photo_id),
            ("data-published-date", article.published_date),
        ] {
            if let Some(value) = value {
                a.insert(key.into(), value.to_string());
            }
        }
        let mut result = self.context.push_tag("article", &a);
        let mut media = MediaData {
            title: article
                .title
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("Related article")
                .into(),
            status: article.author.clone().unwrap_or_default(),
            ..Default::default()
        };
        if let Some(date) = article.published_date.filter(|d| *d != 0) {
            if !media.status.is_empty() {
                media.status += ", ";
            }
            media.status += &format_date_time(date, false, ".", ":", " ", self.dates)?;
        }
        let description = article.description.as_deref().unwrap_or("");
        let mut displaced = false;
        if let Some(id) = article.photo_id {
            let photo = self.message.photos.get(&id).filter(|_| id != 0);
            let unavailable = rich_file_description(photo.map(|p| &p.image.file))?;
            media.classes = "media_photo".into();
            media.description = if unavailable.is_empty() {
                description
            } else {
                unavailable
            }
            .into();
            displaced = !unavailable.is_empty() && !description.is_empty();
            result += &self.media.photo_card(self.context, media, photo)?;
        } else {
            media.classes = "media_file".into();
            media.description = description.into();
            result += &self.media.generic(self.context, media)?;
        }
        if displaced {
            result += &self
                .context
                .push_tag("div", &attrs([("class", "rich_article_description")]));
            result += &serialize_string(description);
            result += &self.context.pop_tag()?;
        }
        result += &self.render_source_meta(&article.url, false)?;
        Ok(result + &self.context.pop_tag()?)
    }
    pub(crate) fn render_related_articles(&mut self, block: &RichBlock) -> Result<String> {
        let title = self.render_text(&block.text)?;
        let mut result = self.context.push_tag(
            "section",
            &rich_block_attributes("rich_related_articles", "related-articles"),
        );
        if !title.is_empty() {
            result += &self.context.push_tag(
                "div",
                &attrs([("class", "rich_related_title"), ("dir", "auto")]),
            );
            result += &title;
            result += &self.context.pop_tag()?;
        }
        result += &self
            .context
            .push_tag("div", &attrs([("class", "rich_related_items")]));
        for (index, article) in block.related_articles.iter().enumerate() {
            result += &self.render_related_article(article, index)?;
        }
        result += &self.context.pop_tag()?;
        Ok(result + &self.context.pop_tag()?)
    }
}
