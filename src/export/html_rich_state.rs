//! Rich message media geometry admission and trailing output classification.
use super::{error::Result, html_rich_support::*, html_rich_text::RichHtmlRenderer, model_rich::*};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichTailState {
    Empty,
    Media,
    Other,
}
impl RichHtmlRenderer<'_> {
    fn media_item_size(&self, block: &RichBlock) -> Option<(i64, i64)> {
        let size = match block.kind {
            RichBlockKind::Photo => {
                let p = self
                    .message
                    .photos
                    .get(&block.photo_id)
                    .filter(|_| block.photo_id != 0)?;
                if p.image.file.relative_path.is_empty() {
                    return None;
                }
                (p.image.width, p.image.height)
            }
            RichBlockKind::Video => {
                let d = self
                    .message
                    .documents
                    .get(&block.document_id)
                    .filter(|_| block.document_id != 0)?;
                if d.file.relative_path.is_empty() || d.thumb.file.relative_path.is_empty() {
                    return None;
                }
                (d.width, d.height)
            }
            _ => return None,
        };
        (size.0 > 0 && size.1 > 0).then_some((i64::from(size.0), i64::from(size.1)))
    }
    pub(crate) fn media_group_sizes(&self, blocks: &[RichBlock]) -> Vec<(i64, i64)> {
        let mut result = Vec::new();
        for block in blocks {
            let Some(size) = self.media_item_size(block) else {
                return vec![];
            };
            if rich_caption_has_output(&block.caption)
                || block.optional_url.as_deref().is_some_and(|u| !u.is_empty())
            {
                return vec![];
            }
            result.push(size);
        }
        result
    }
    pub(crate) fn tail_state_of(&self, blocks: &[RichBlock]) -> Result<RichTailState> {
        for block in blocks.iter().rev() {
            let state = self.tail_state(block)?;
            if state != RichTailState::Empty {
                return Ok(state);
            }
        }
        Ok(RichTailState::Empty)
    }
    fn tail_state(&self, block: &RichBlock) -> Result<RichTailState> {
        use RichBlockKind as K;
        use RichTailState as S;
        Ok(match block.kind {
            K::Unsupported
            | K::Heading
            | K::Paragraph
            | K::Footer
            | K::Thinking
            | K::AuthorDate
            | K::Code
            | K::Divider
            | K::Math
            | K::Table
            | K::ButtonRow
            | K::Unknown => S::Other,
            K::Anchor => S::Empty,
            K::List => {
                if let Some(item) = block.list_items.last() {
                    if item.content == RichListItemContent::Blocks {
                        let state = self.tail_state_of(&item.blocks)?;
                        if state != S::Empty {
                            return Ok(state);
                        }
                    }
                    S::Other
                } else {
                    S::Empty
                }
            }
            K::Quote => {
                if rich_text_has_output(&block.quote_caption) {
                    S::Other
                } else {
                    if block.quote_content == RichQuoteContent::Blocks {
                        let state = self.tail_state_of(&block.blocks)?;
                        if state != S::Empty {
                            return Ok(state);
                        }
                    }
                    S::Other
                }
            }
            K::Photo => {
                if rich_caption_has_output(&block.caption)
                    || block.optional_url.as_deref().is_some_and(|u| !u.is_empty())
                {
                    S::Other
                } else {
                    S::Media
                }
            }
            K::Video | K::Audio | K::File | K::Map | K::InputMap => {
                if rich_caption_has_output(&block.caption) {
                    S::Other
                } else {
                    S::Media
                }
            }
            K::Cover => self.tail_state_of(&block.blocks)?,
            K::Embed => {
                if rich_caption_has_output(&block.caption)
                    || block.poster_photo_id.is_some()
                        && block.optional_url.as_deref().is_some_and(|u| !u.is_empty())
                {
                    S::Other
                } else {
                    S::Media
                }
            }
            K::EmbedPost => {
                if rich_caption_has_output(&block.caption) {
                    S::Other
                } else {
                    let state = self.tail_state_of(&block.blocks)?;
                    if state != S::Empty {
                        state
                    } else if block.url.is_empty() {
                        S::Media
                    } else {
                        S::Other
                    }
                }
            }
            K::Collage | K::Slideshow => {
                if rich_caption_has_output(&block.caption) {
                    S::Other
                } else {
                    self.tail_state_of(&block.blocks)?
                }
            }
            K::Channel => S::Media,
            K::Details => {
                if block.open {
                    let state = self.tail_state_of(&block.blocks)?;
                    if state != S::Empty {
                        return Ok(state);
                    }
                }
                S::Other
            }
            K::RelatedArticles => {
                if let Some(article) = block.related_articles.last() {
                    if !article.url.is_empty() {
                        S::Other
                    } else if article.photo_id.is_some()
                        && article
                            .description
                            .as_deref()
                            .is_some_and(|d| !d.is_empty())
                    {
                        let photo = article
                            .photo_id
                            .and_then(|id| self.message.photos.get(&id).filter(|_| id != 0));
                        if !rich_file_description(photo.map(|p| &p.image.file))?.is_empty() {
                            S::Other
                        } else {
                            S::Media
                        }
                    } else {
                        S::Media
                    }
                } else if rich_text_has_output(&block.text) {
                    S::Other
                } else {
                    S::Empty
                }
            }
        })
    }
}
