//! Complete Desktop rich block JSON serialization.
use super::{
    error::{ExportError, Result},
    json_rich::*,
    json_rich_metadata::*,
    json_serialize::*,
    model_rich::*,
};
pub fn serialize_rich_block(
    context: &mut RichSerializeContext<'_>,
    data: &RichBlock,
) -> Result<Vec<u8>> {
    let kind = if data.kind == RichBlockKind::Unknown {
        "unsupported".into()
    } else {
        enum_name(data.kind)
    };
    object(
        context,
        vec![("type".into(), serialize_string(kind))],
        |context, values| {
            use RichBlockKind as K;
            let photo_keys = [
                "photo",
                "photo_skip_reason",
                "photo_file_size",
                "width",
                "height",
            ];
            match data.kind {
                K::Unsupported => {
                    push(
                        values,
                        "source_type",
                        serialize_string("page_block_unsupported"),
                    );
                    push(values, "unsupported", b"true".to_vec());
                }
                K::Heading => {
                    push(values, "level", number(data.heading_level));
                    push(values, "text", serialize_rich_text(context, &data.text)?);
                }
                K::Paragraph | K::Footer | K::Thinking => {
                    push(values, "text", serialize_rich_text(context, &data.text)?)
                }
                K::AuthorDate => {
                    push(values, "author", serialize_rich_text(context, &data.text)?);
                    push(values, "date", serialize_date(data.date, context.dates)?);
                    push(values, "date_unixtime", serialize_date_raw(data.date));
                }
                K::Code => {
                    push(values, "text", serialize_rich_text(context, &data.text)?);
                    push(values, "language", serialize_string(&data.language));
                }
                K::Divider => {}
                K::Anchor => push(values, "name", serialize_string(&data.name)),
                K::List => {
                    push(values, "kind", serialize_string(enum_name(data.list_kind)));
                    if data.list_kind == RichListKind::Ordered {
                        push(values, "reversed", boolean(data.ordered_list.reversed));
                        if let Some(start) = data.ordered_list.start {
                            push(values, "start", number(start));
                        }
                        if let Some(kind) = &data.ordered_list.r#type {
                            push(values, "list_type", serialize_string(kind));
                        }
                    }
                    push(
                        values,
                        "items",
                        rich_array(context, &data.list_items, list_item)?,
                    );
                }
                K::Quote => {
                    push(
                        values,
                        "content",
                        serialize_string(enum_name(data.quote_content)),
                    );
                    push(values, "pullquote", boolean(data.pullquote));
                    if data.quote_content == RichQuoteContent::Text {
                        push(values, "text", serialize_rich_text(context, &data.text)?);
                    } else {
                        push(
                            values,
                            "blocks",
                            serialize_rich_blocks(context, &data.blocks)?,
                        );
                    }
                    push(
                        values,
                        "caption",
                        serialize_rich_text(context, &data.quote_caption)?,
                    );
                }
                K::Photo => {
                    push(values, "photo_id", id(data.photo_id));
                    append_photo_metadata(
                        values,
                        context.message.photos.get(&data.photo_id),
                        photo_keys,
                    );
                    push(values, "spoiler", boolean(data.spoiler));
                    if let Some(url) = &data.optional_url {
                        push(values, "url", serialize_string(url));
                    }
                    if let Some(value) = data.optional_webpage_id {
                        push(values, "webpage_id", id(value));
                    }
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::Video => {
                    push(values, "document_id", id(data.document_id));
                    append_document_metadata(
                        values,
                        context.message.documents.get(&data.document_id),
                        true,
                    );
                    push(values, "autoplay", boolean(data.autoplay));
                    push(values, "loop", boolean(data.r#loop));
                    push(values, "spoiler", boolean(data.spoiler));
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::Cover => {
                    if data.blocks.len() != 1 {
                        return Err(ExportError::InvalidRecord);
                    }
                    push(
                        values,
                        "block",
                        serialize_rich_block(context, &data.blocks[0])?,
                    );
                }
                K::Embed => {
                    if let Some(url) = &data.optional_url {
                        push(values, "url", serialize_string(url));
                    }
                    if let Some(html) = &data.html {
                        push(values, "html", serialize_string(html));
                    }
                    if let Some(photo) = data.poster_photo_id {
                        push(values, "poster_photo_id", id(photo));
                        append_photo_metadata(
                            values,
                            context.message.photos.get(&photo),
                            [
                                "poster_photo",
                                "poster_photo_skip_reason",
                                "poster_photo_file_size",
                                "poster_photo_width",
                                "poster_photo_height",
                            ],
                        );
                    }
                    if let Some(value) = data.width {
                        push(values, "width", number(value));
                    }
                    if let Some(value) = data.height {
                        push(values, "height", number(value));
                    }
                    push(values, "full_width", boolean(data.full_width));
                    push(values, "allow_scrolling", boolean(data.allow_scrolling));
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::EmbedPost => {
                    push(values, "url", serialize_string(&data.url));
                    push(values, "webpage_id", id(data.webpage_id));
                    push(values, "author_photo_id", id(data.author_photo_id));
                    append_photo_metadata(
                        values,
                        context.message.photos.get(&data.author_photo_id),
                        [
                            "author_photo",
                            "author_photo_skip_reason",
                            "author_photo_file_size",
                            "author_photo_width",
                            "author_photo_height",
                        ],
                    );
                    push(values, "author", serialize_string(&data.author));
                    push(values, "date", serialize_date(data.date, context.dates)?);
                    push(values, "date_unixtime", serialize_date_raw(data.date));
                    push(
                        values,
                        "blocks",
                        serialize_rich_blocks(context, &data.blocks)?,
                    );
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::Collage | K::Slideshow => {
                    push(
                        values,
                        "items",
                        serialize_rich_blocks(context, &data.blocks)?,
                    );
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::Channel => push(values, "channel", channel(context, &data.channel)),
                K::Audio | K::File => {
                    push(values, "document_id", id(data.document_id));
                    append_document_metadata(
                        values,
                        context.message.documents.get(&data.document_id),
                        true,
                    );
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::Math => push(values, "formula", serialize_string(&data.formula)),
                K::Table => {
                    push(values, "title", serialize_rich_text(context, &data.text)?);
                    push(values, "bordered", boolean(data.bordered));
                    push(values, "striped", boolean(data.striped));
                    push(values, "compact", boolean(data.compact));
                    push(
                        values,
                        "rows",
                        rich_array(context, &data.table_rows, table_row)?,
                    );
                }
                K::Details => {
                    push(values, "title", serialize_rich_text(context, &data.text)?);
                    push(values, "open", boolean(data.open));
                    push(
                        values,
                        "blocks",
                        serialize_rich_blocks(context, &data.blocks)?,
                    );
                }
                K::RelatedArticles => {
                    push(values, "title", serialize_rich_text(context, &data.text)?);
                    push(
                        values,
                        "articles",
                        rich_array(context, &data.related_articles, related_article)?,
                    );
                }
                K::Map | K::InputMap => {
                    push(values, "geo", map_point(context, &data.map_point));
                    push(values, "zoom", number(data.zoom));
                    push(values, "width", number(data.map_width));
                    push(values, "height", number(data.map_height));
                    push(values, "caption", caption(context, &data.caption)?);
                }
                K::ButtonRow => {
                    push(
                        values,
                        "alignment",
                        serialize_string(enum_name(data.button_alignment)),
                    );
                    push(
                        values,
                        "buttons",
                        rich_array(context, &data.buttons, rich_button)?,
                    );
                }
                K::Unknown => {
                    push(values, "source_type", serialize_string("unknown"));
                    push(values, "unsupported", b"true".to_vec());
                }
            }
            Ok(())
        },
    )
}
pub fn serialize_rich_blocks(
    context: &mut RichSerializeContext<'_>,
    data: &[RichBlock],
) -> Result<Vec<u8>> {
    rich_array(context, data, serialize_rich_block)
}
