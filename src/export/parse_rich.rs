//! Rich-message media maps, exact reference refresh and ordered visitor traversal.
use super::{
    error::Result,
    format::DateContext,
    model::*,
    model_document::{parse_document, parse_photo, refresh_file_reference},
    model_peers::type_is,
    model_rich::*,
    parse_rich_block::parse_rich_blocks,
};
use serde_json::Value;
use std::collections::BTreeMap;
pub fn parse_rich_message(
    context: &mut ParseMediaContext,
    data: &Value,
    folder: &str,
    date: i64,
    dates: &DateContext,
) -> Result<RichMessage> {
    let mut result = RichMessage {
        rtl: boolean(data, "rtl"),
        part: boolean(data, "part"),
        ..Default::default()
    };
    for photo in array(data, "photos") {
        context.photos += 1;
        let photo = parse_photo(
            photo,
            &format!(
                "{folder}photos/{}",
                prepare_photo_file_name(context.photos, date, dates)?
            ),
        );
        result.photos.insert(photo.id, photo);
    }
    for document in array(data, "documents") {
        let document = parse_document(context, document, folder, date, dates)?;
        result.documents.insert(document.id, document);
    }
    result.blocks = parse_rich_blocks(array(data, "blocks"));
    Ok(result)
}
pub fn extract_full_rich_message(messages: &[Value], message_id: i64) -> Option<Value> {
    let mut matches = 0;
    let mut rich = None;
    for entry in messages {
        if integer(entry, "id") != message_id {
            continue;
        }
        matches += 1;
        if type_is(entry, "Message") {
            let candidate = &entry["rich_message"];
            if !candidate.is_null() && !boolean(candidate, "part") {
                rich = Some(candidate.clone());
            }
        }
    }
    if matches == 1 { rich } else { None }
}
pub fn refresh_rich_message_file_reference(
    location: &FileLocation,
    message: &RichMessage,
) -> Option<FileLocation> {
    let mut refreshed = location.clone();
    for photo in message.photos.values() {
        if refresh_file_reference(&mut refreshed, &photo.image.file.location) {
            return Some(refreshed);
        }
    }
    for document in message.documents.values() {
        if refresh_file_reference(&mut refreshed, &document.file.location) {
            return Some(refreshed);
        }
        if document.thumb.width > 0
            && refresh_file_reference(&mut refreshed, &document.thumb.file.location)
        {
            return Some(refreshed);
        }
    }
    None
}
pub trait RichVisitor {
    fn text(&mut self, _text: &mut RichText) {}
    fn photo(&mut self, _photo: &mut Photo) {}
    fn document(&mut self, _document: &mut Document) {}
}
struct VisitState<'a> {
    photos: &'a mut BTreeMap<i64, Photo>,
    documents: &'a mut BTreeMap<i64, Document>,
    visitor: &'a mut dyn RichVisitor,
}
impl VisitState<'_> {
    fn photo(&mut self, id: i64) {
        if let Some(photo) = self.photos.get_mut(&id) {
            self.visitor.photo(photo);
        }
    }
    fn document(&mut self, id: i64) {
        if let Some(document) = self.documents.get_mut(&id) {
            self.visitor.document(document);
        }
    }
    fn text(&mut self, text: &mut RichText) {
        self.visitor.text(text);
        if text.r#type == RichTextType::InlineImage {
            self.document(text.id);
        }
        for child in &mut text.children {
            self.text(child);
        }
        for child in &mut text.old_children {
            self.text(child);
        }
    }
    fn caption(&mut self, caption: &mut RichCaption) {
        self.text(&mut caption.text);
        self.text(&mut caption.credit);
    }
    fn blocks(&mut self, blocks: &mut [RichBlock]) {
        for block in blocks {
            self.block(block);
        }
    }
    fn block(&mut self, block: &mut RichBlock) {
        use RichBlockKind as K;
        match block.kind {
            K::Heading | K::Paragraph | K::Footer | K::Thinking | K::AuthorDate | K::Code => {
                self.text(&mut block.text)
            }
            K::List => {
                for item in &mut block.list_items {
                    if item.content == RichListItemContent::Text {
                        if let Some(text) = &mut item.text {
                            self.text(text);
                        }
                    } else {
                        self.blocks(&mut item.blocks);
                    }
                }
            }
            K::Quote => {
                if block.quote_content == RichQuoteContent::Text {
                    self.text(&mut block.text);
                } else {
                    self.blocks(&mut block.blocks);
                }
                self.text(&mut block.quote_caption);
            }
            K::Photo => {
                self.photo(block.photo_id);
                self.caption(&mut block.caption);
            }
            K::Video | K::Audio | K::File => {
                self.document(block.document_id);
                self.caption(&mut block.caption);
            }
            K::Cover => {
                if let Some(first) = block.blocks.first_mut() {
                    self.block(first);
                }
            }
            K::Embed => {
                if let Some(id) = block.poster_photo_id {
                    self.photo(id);
                }
                self.caption(&mut block.caption);
            }
            K::EmbedPost => {
                self.photo(block.author_photo_id);
                self.blocks(&mut block.blocks);
                self.caption(&mut block.caption);
            }
            K::Collage | K::Slideshow => {
                self.blocks(&mut block.blocks);
                self.caption(&mut block.caption);
            }
            K::Table => {
                self.text(&mut block.text);
                for row in &mut block.table_rows {
                    for cell in &mut row.cells {
                        if let Some(text) = &mut cell.text {
                            self.text(text);
                        }
                    }
                }
            }
            K::Details => {
                self.text(&mut block.text);
                self.blocks(&mut block.blocks);
            }
            K::RelatedArticles => {
                self.text(&mut block.text);
                for article in &block.related_articles {
                    if let Some(id) = article.photo_id {
                        self.photo(id);
                    }
                }
            }
            K::Map | K::InputMap => self.caption(&mut block.caption),
            K::ButtonRow => {
                for button in &mut block.buttons {
                    self.text(button);
                }
            }
            _ => {}
        }
    }
}
pub fn visit_rich_message(message: &mut RichMessage, visitor: &mut dyn RichVisitor) {
    VisitState {
        photos: &mut message.photos,
        documents: &mut message.documents,
        visitor,
    }
    .blocks(&mut message.blocks);
}
