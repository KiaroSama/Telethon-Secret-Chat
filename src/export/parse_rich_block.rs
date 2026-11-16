//! Native TL page block parsing.
use super::{
    model::{array, boolean, integer, kind, string},
    model_peers::type_is,
    model_rich::*,
    parse_rich_text::*,
};
use serde_json::Value;
fn list_item(item: &Value, ordered: bool) -> RichListItem {
    let mut r = RichListItem {
        task_state: if !boolean(item, "checkbox") {
            RichTaskState::None_
        } else if boolean(item, "checked") {
            RichTaskState::Checked
        } else {
            RichTaskState::Unchecked
        },
        ..Default::default()
    };
    if ordered {
        r.num = item["num"].as_str().map(str::to_owned);
        r.value = item["value"].as_i64();
        r.r#type = item["type"].as_str().map(str::to_owned);
    }
    if type_is(item, "PageListItemBlocks") || type_is(item, "PageListOrderedItemBlocks") {
        r.content = RichListItemContent::Blocks;
        r.blocks = parse_rich_blocks(array(item, "blocks"));
    } else {
        r.text = Some(parse_rich_text(&item["text"]));
    }
    r
}
fn table_cell(data: &Value) -> RichTableCell {
    RichTableCell {
        header: boolean(data, "header"),
        alignment: if boolean(data, "align_right") {
            RichTableAlignment::Right
        } else if boolean(data, "align_center") {
            RichTableAlignment::Center
        } else {
            RichTableAlignment::Left
        },
        vertical_alignment: if boolean(data, "valign_bottom") {
            RichTableVerticalAlignment::Bottom
        } else if boolean(data, "valign_middle") {
            RichTableVerticalAlignment::Middle
        } else {
            RichTableVerticalAlignment::Top
        },
        text: if data["text"].is_null() {
            None
        } else {
            Some(parse_rich_text(&data["text"]))
        },
        colspan: data["colspan"].as_i64(),
        rowspan: data["rowspan"].as_i64(),
    }
}
fn related_article(data: &Value) -> RichRelatedArticle {
    RichRelatedArticle {
        url: string(data, "url").into(),
        webpage_id: integer(data, "webpage_id"),
        title: data["title"].as_str().map(str::to_owned),
        description: data["description"].as_str().map(str::to_owned),
        author: data["author"].as_str().map(str::to_owned),
        photo_id: data["photo_id"].as_i64(),
        published_date: data["published_date"].as_i64(),
    }
}
fn text_block(kind: RichBlockKind, text: &Value) -> RichBlock {
    RichBlock {
        kind,
        text: parse_rich_text(text),
        ..Default::default()
    }
}
pub fn parse_rich_block(block: &Value) -> RichBlock {
    let raw = kind(block);
    let name = raw
        .trim_start_matches("PageBlock")
        .trim_start_matches("pageBlock");
    let level = match name {
        "Title" | "Heading1" => 1,
        "Subtitle" | "Heading2" => 2,
        "Header" | "Heading3" => 3,
        "Subheader" | "Heading4" => 4,
        "Kicker" | "Heading5" => 5,
        "Heading6" => 6,
        _ => 0,
    };
    if level != 0 {
        return RichBlock {
            heading_level: level,
            ..text_block(RichBlockKind::Heading, &block["text"])
        };
    }
    match name {
        "Paragraph" => text_block(RichBlockKind::Paragraph, &block["text"]),
        "Footer" => text_block(RichBlockKind::Footer, &block["text"]),
        "Thinking" => text_block(RichBlockKind::Thinking, &block["text"]),
        "Unsupported" => RichBlock {
            kind: RichBlockKind::Unsupported,
            unsupported: true,
            ..Default::default()
        },
        "AuthorDate" => RichBlock {
            date: integer(block, "published_date"),
            ..text_block(RichBlockKind::AuthorDate, &block["author"])
        },
        "Preformatted" => RichBlock {
            language: string(block, "language").into(),
            ..text_block(RichBlockKind::Code, &block["text"])
        },
        "Divider" => RichBlock {
            kind: RichBlockKind::Divider,
            ..Default::default()
        },
        "Anchor" => RichBlock {
            kind: RichBlockKind::Anchor,
            name: string(block, "name").into(),
            ..Default::default()
        },
        "Cover" => RichBlock {
            kind: RichBlockKind::Cover,
            blocks: vec![parse_rich_block(&block["cover"])],
            ..Default::default()
        },
        "Channel" => RichBlock {
            kind: RichBlockKind::Channel,
            channel: parse_rich_channel(&block["channel"]),
            ..Default::default()
        },
        "Math" => RichBlock {
            kind: RichBlockKind::Math,
            formula: string(block, "source").into(),
            ..Default::default()
        },
        "Photo" => RichBlock {
            kind: RichBlockKind::Photo,
            photo_id: integer(block, "photo_id"),
            caption: parse_rich_caption(&block["caption"]),
            spoiler: boolean(block, "spoiler"),
            optional_url: block["url"].as_str().map(str::to_owned),
            optional_webpage_id: block["webpage_id"].as_i64(),
            ..Default::default()
        },
        "Video" => RichBlock {
            kind: RichBlockKind::Video,
            document_id: integer(block, "video_id"),
            caption: parse_rich_caption(&block["caption"]),
            autoplay: boolean(block, "autoplay"),
            r#loop: boolean(block, "loop"),
            spoiler: boolean(block, "spoiler"),
            ..Default::default()
        },
        "Audio" | "Document" => RichBlock {
            kind: if name == "Audio" {
                RichBlockKind::Audio
            } else {
                RichBlockKind::File
            },
            document_id: integer(
                block,
                if name == "Audio" {
                    "audio_id"
                } else {
                    "document_id"
                },
            ),
            caption: parse_rich_caption(&block["caption"]),
            ..Default::default()
        },
        "Embed" => RichBlock {
            kind: RichBlockKind::Embed,
            full_width: boolean(block, "full_width"),
            allow_scrolling: boolean(block, "allow_scrolling"),
            caption: parse_rich_caption(&block["caption"]),
            optional_url: block["url"].as_str().map(str::to_owned),
            html: block["html"].as_str().map(str::to_owned),
            poster_photo_id: block["poster_photo_id"].as_i64(),
            width: block["w"].as_i64(),
            height: block["h"].as_i64(),
            ..Default::default()
        },
        "EmbedPost" => RichBlock {
            kind: RichBlockKind::EmbedPost,
            url: string(block, "url").into(),
            webpage_id: integer(block, "webpage_id"),
            author_photo_id: integer(block, "author_photo_id"),
            author: string(block, "author").into(),
            date: integer(block, "date"),
            blocks: parse_rich_blocks(array(block, "blocks")),
            caption: parse_rich_caption(&block["caption"]),
            ..Default::default()
        },
        "Collage" | "Slideshow" => RichBlock {
            kind: if name == "Collage" {
                RichBlockKind::Collage
            } else {
                RichBlockKind::Slideshow
            },
            blocks: parse_rich_blocks(array(block, "items")),
            caption: parse_rich_caption(&block["caption"]),
            ..Default::default()
        },
        "Map" => RichBlock {
            kind: RichBlockKind::Map,
            map_point: parse_rich_map_point(&block["geo"]),
            zoom: integer(block, "zoom"),
            map_width: integer(block, "w"),
            map_height: integer(block, "h"),
            caption: parse_rich_caption(&block["caption"]),
            ..Default::default()
        },
        "List" | "OrderedList" => RichBlock {
            kind: RichBlockKind::List,
            list_kind: if name == "List" {
                RichListKind::Bullet
            } else {
                RichListKind::Ordered
            },
            ordered_list: if name == "List" {
                RichOrderedList::default()
            } else {
                RichOrderedList {
                    r#type: block["type"].as_str().map(str::to_owned),
                    start: block["start"].as_i64(),
                    reversed: boolean(block, "reversed"),
                }
            },
            list_items: array(block, "items")
                .iter()
                .map(|i| list_item(i, name == "OrderedList"))
                .collect(),
            ..Default::default()
        },
        "Blockquote" | "Pullquote" => RichBlock {
            quote_caption: parse_rich_text(&block["caption"]),
            pullquote: name == "Pullquote",
            ..text_block(RichBlockKind::Quote, &block["text"])
        },
        "BlockquoteBlocks" => RichBlock {
            kind: RichBlockKind::Quote,
            quote_content: RichQuoteContent::Blocks,
            blocks: parse_rich_blocks(array(block, "blocks")),
            quote_caption: parse_rich_text(&block["caption"]),
            ..Default::default()
        },
        "Table" => RichBlock {
            bordered: boolean(block, "bordered"),
            striped: boolean(block, "striped"),
            compact: boolean(block, "compact"),
            table_rows: array(block, "rows")
                .iter()
                .map(|r| RichTableRow {
                    cells: array(r, "cells").iter().map(table_cell).collect(),
                })
                .collect(),
            ..text_block(RichBlockKind::Table, &block["title"])
        },
        "Details" => RichBlock {
            open: boolean(block, "open"),
            blocks: parse_rich_blocks(array(block, "blocks")),
            ..text_block(RichBlockKind::Details, &block["title"])
        },
        "RelatedArticles" => RichBlock {
            kind: RichBlockKind::RelatedArticles,
            text: parse_rich_text(&block["title"]),
            related_articles: array(block, "articles")
                .iter()
                .map(related_article)
                .collect(),
            ..Default::default()
        },
        "ButtonRow" => RichBlock {
            kind: RichBlockKind::ButtonRow,
            button_alignment: if boolean(block, "align_left") {
                RichButtonAlignment::Left
            } else if boolean(block, "align_center") {
                RichButtonAlignment::Center
            } else if boolean(block, "align_right") {
                RichButtonAlignment::Right
            } else {
                RichButtonAlignment::Stretch
            },
            buttons: array(block, "buttons")
                .iter()
                .filter_map(|b| parse_rich_button(&b["text"], &b["type"], &b["style"], false))
                .collect(),
            ..Default::default()
        },
        _ => {
            if type_is(block, "InputPageBlockMap") {
                RichBlock {
                    kind: RichBlockKind::InputMap,
                    map_point: parse_rich_map_point(&block["geo"]),
                    zoom: integer(block, "zoom"),
                    map_width: integer(block, "w"),
                    map_height: integer(block, "h"),
                    caption: parse_rich_caption(&block["caption"]),
                    ..Default::default()
                }
            } else {
                RichBlock::default()
            }
        }
    }
}
pub fn parse_rich_blocks(blocks: &[Value]) -> Vec<RichBlock> {
    blocks.iter().map(parse_rich_block).collect()
}
