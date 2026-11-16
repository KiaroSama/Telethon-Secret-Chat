#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::{
    error::{ExportError, Result},
    format::DateContext,
    html_rich::{RichMediaCallbacks, render_rich_message},
    html_rich_support::MediaData,
    html_text::HtmlContext,
    model::{Document, Photo},
    model_rich::*,
};
use serde_json::json;
struct MapCallback;
impl RichMediaCallbacks for MapCallback {
    fn generic(&mut self, _: &mut HtmlContext, data: MediaData) -> Result<String> {
        Ok(format!(
            "[generic {}|{}|{}|{}|{}]",
            data.title, data.description, data.status, data.classes, data.link
        ))
    }
    fn photo(&mut self, _: &mut HtmlContext, _: Option<&Photo>) -> Result<String> {
        Err(ExportError::Unsupported)
    }
    fn video(&mut self, _: &mut HtmlContext, _: Option<&Document>) -> Result<String> {
        Err(ExportError::Unsupported)
    }
    fn audio(&mut self, _: &mut HtmlContext, _: Option<&Document>) -> Result<String> {
        Err(ExportError::Unsupported)
    }
    fn file(&mut self, _: &mut HtmlContext, _: Option<&Document>) -> Result<String> {
        Err(ExportError::Unsupported)
    }
    fn photo_card(
        &mut self,
        _: &mut HtmlContext,
        _: MediaData,
        _: Option<&Photo>,
    ) -> Result<String> {
        Err(ExportError::Unsupported)
    }
}
#[test]
fn malformed_button_and_unknown_block_kind_raise() {
    let broken = RichText {
        r#type: RichTextType::Button,
        children: vec![],
        button: Some(RichButtonPayload::default()),
        ..Default::default()
    };
    let message = RichMessage {
        blocks: vec![RichBlock {
            kind: RichBlockKind::ButtonRow,
            buttons: vec![broken],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut context = HtmlContext::default();
    assert!(matches!(
        render_rich_message(
            &mut context,
            &message,
            5,
            "https://t.me/",
            "",
            &mut MapCallback,
            &DateContext::utc()
        ),
        Err(ExportError::InvalidRecord)
    ));
    assert!(context.empty());
    assert!(serde_json::from_value::<RichBlock>(json!({"kind":"not a kind"})).is_err());
}
fn invalid_map(latitude: f64, longitude: f64) {
    let point = RichMapPoint {
        latitude,
        longitude,
        source: RichMapPointSource::GeoPoint,
        ..Default::default()
    };
    let block = RichBlock {
        kind: RichBlockKind::InputMap,
        map_point: point,
        caption: RichCaption {
            text: RichText {
                r#type: RichTextType::Plain,
                text: "here".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let message = RichMessage {
        blocks: vec![block],
        ..Default::default()
    };
    let html = render_rich_message(
        &mut HtmlContext::default(),
        &message,
        5,
        "https://t.me/",
        "",
        &mut MapCallback,
        &DateContext::utc(),
    )
    .unwrap();
    assert!(html.contains("data-rich-kind=\"input-map\""));
    assert!(html.contains("[generic Location|||media_location|]"));
    assert!(html.contains("<div class=\"rich_caption_text\">\nhere\n"));
}
#[test]
fn map_nan_has_no_status_or_link() {
    invalid_map(f64::NAN, 0.0);
}
#[test]
fn map_infinity_has_no_status_or_link() {
    invalid_map(0.0, f64::INFINITY);
}
