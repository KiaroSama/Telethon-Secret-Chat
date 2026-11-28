//! Independent public-data assertions for audited Desktop v7.4.4 behavior.
#![allow(clippy::unwrap_used, dead_code)]
use grammers_tl_types::{enums as api, types as raw};
use kiaro_secret_chat::export::{
    format::{DateContext, format_grams_amount},
    html_rich_media::NativeRichMedia,
    html_rich_text::RichHtmlRenderer,
    html_service,
    html_text::{self, HtmlContext, PeersMap, WrapBase},
    json_message,
    json_rich::{self, RichSerializeContext},
    json_serialize::{self, JsonContext},
    model::*,
    model_actions::*,
    model_dialogs::DialogInfo,
    model_message::{HistoryMessageMarkupButton, MarkupButtonType, Message},
    model_rich::{RichMapPoint, RichMapPointSource, RichMessage, RichTextType},
    parse_actions::parse_service_action,
    parse_media::parse_geo_point,
    parse_rich_text::parse_rich_text,
    tl_value,
};
use serde_json::{Value, json};
fn service(action: Action, out: bool, peer: i64) -> (String, Value) {
    let message = Message {
        id: 7,
        date: 1700000000,
        out,
        action: ServiceAction {
            content: Some(action),
        },
        ..Default::default()
    };
    let dialog = DialogInfo {
        peer_id: peer,
        ..Default::default()
    };
    let dates = DateContext::utc();
    let html = String::from_utf8(
        html_service::service_text(
            &message,
            &dialog,
            &PeersMap::default(),
            "https://t.me/",
            "",
            str::to_owned,
            &dates,
        )
        .unwrap(),
    )
    .unwrap();
    let bytes = json_message::serialize_message(
        &mut JsonContext::default(),
        &message,
        &Default::default(),
        "https://t.me/",
        &dates,
    )
    .unwrap();
    (html, serde_json::from_slice(&bytes).unwrap())
}
fn parse(value: Value) -> Action {
    parse_service_action(
        &mut ParseMediaContext::default(),
        &value,
        "",
        1700000000,
        &DateContext::utc(),
    )
    .unwrap()
    .content
    .unwrap()
}
pub fn json_safety() {
    for byte in 0_u8..32 {
        let expected = match byte {
            b'\n' => b"\"\\n\"".to_vec(),
            b'\r' => b"\"\\r\"".to_vec(),
            b'\t' => b"\"\\t\"".to_vec(),
            other => format!("\"\\u00{other:02X}\"").into_bytes(),
        };
        assert_eq!(json_serialize::serialize_string([byte]), expected);
        assert_eq!(
            serde_json::from_slice::<String>(&expected)
                .unwrap()
                .as_bytes(),
            [byte]
        );
    }
    for (text, expected) in [
        ("x\u{2028}y", b"\"x\\u2028y\"".as_slice()),
        ("x\u{2029}y", b"\"x\\u2029y\"".as_slice()),
    ] {
        let bytes = json_serialize::serialize_string(text);
        assert_eq!(bytes, expected);
        assert_eq!(serde_json::from_slice::<String>(&bytes).unwrap(), text);
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(json_serialize::float_allow_null(value), b"null");
    }
    assert_eq!(json_serialize::float_allow_null(1.25), b"1.250000");
}
pub fn model_actions() {
    let raw: api::MessageAction = raw::MessageActionGramTransfer {
        comment_encrypted: false,
        onramp_provider_name: None,
        amount: -1_234_567_890,
        peer_address: "<peer>".into(),
        transaction_id: "tx\"<&".into(),
        comment: Some("<script>&".into()),
    }
    .into();
    let action = parse(tl_value::to_value(&raw).unwrap());
    let (html, value) = service(action, false, peer_from_user(333000));
    assert_eq!(
        html,
        "Someone (&lt;peer&gt;) sent you -1.23456789 Grams, transaction tx&quot;&lt;&amp;, with comment: &laquo;&lt;script&gt;&amp;&raquo;"
    );
    assert_eq!(value["action"], "gram_transfer");
    assert_eq!(value["amount_whole"], -1);
    assert_eq!(value["amount_nano"], -234567890);
    assert_eq!(value["comment"], "<script>&");
    assert_eq!(value["comment_encrypted"], false);
    assert!(value.get("encrypted_comment_base64").is_none());
    let action = Action::GramTransfer(ActionGramTransfer {
        amount: 1_000_000_000,
        comment_encrypted: true,
        comment: "a\"<&".into(),
        ..Default::default()
    });
    let (html, value) = service(action, true, peer_from_user(7));
    assert_eq!(
        html,
        "You sent 1 Gram to Deleted, with an <span class=\"gram_transfer_encrypted_comment\" data-encrypted-comment=\"a&quot;&lt;&amp;\">encrypted comment</span>"
    );
    assert_eq!(value["encrypted_comment_base64"], "a\"<&");
    assert!(value.get("comment").is_none());
    for (accepted, declined, word) in [
        (false, false, "received"),
        (true, false, "approved"),
        (false, true, "declined"),
        (true, true, "approved"),
    ] {
        let raw: api::MessageAction = raw::MessageActionWalletTonConnectRequest {
            accepted,
            declined,
            session_id: -1,
            expires: 1700000010,
            topic: Some("<topic>".into()),
            trace_id: Some("public-trace".into()),
            dapp_name: None,
        }
        .into();
        let (html, value) = service(parse(tl_value::to_value(&raw).unwrap()), false, 0);
        assert_eq!(
            html,
            format!("You {word} a TON Connect request &laquo;&lt;topic&gt;&raquo;.")
        );
        assert_eq!(value["session_id"], json!(u64::MAX));
        assert_eq!(value["action"], "ton_connect_request");
        assert_eq!(value["expires"], 1700000010);
        assert_eq!(value["accepted"], accepted);
        assert_eq!(value["declined"], declined);
    }
    let action = parse(tl_value::to_value(&api::MessageAction::NoPaidMessagesTokenUsed).unwrap());
    let (html, value) = service(action, false, 0);
    assert_eq!(html, "Deleted started a chat via a free messages link");
    assert_eq!(value["action"], "no_paid_messages_token_used");
    for (input, expected) in [
        (0, "0 Grams"),
        (1, "0.000000001 Grams"),
        (-1, "-0.000000001 Grams"),
        (1_000_000_000, "1 Gram"),
        (-1_000_000_000, "-1 Gram"),
        (1_230_000_000, "1.23 Grams"),
        (i64::MIN, "-9223372036.854775808 Grams"),
        (i64::MAX, "9223372036.854775807 Grams"),
    ] {
        assert_eq!(format_grams_amount(input), expected);
    }
    let parts = parse_text(
        "EQ<address>",
        &[json!({"_":"MessageEntityTonAddress","offset":0,"length":11})],
    );
    assert_eq!(parts[0].kind, TextType::TonAddress);
    assert_eq!(
        html_text::format_text(&parts, "https://t.me/", ""),
        b"EQ&lt;address&gt;"
    );
    assert_eq!(json_serialize::text_type_name(parts[0].kind), "ton_address");
    let text = parse_rich_text(
        &json!({"_":"TextTonAddress","text":{"_":"TextPlain","text":"EQ<address>"}}),
    );
    assert_eq!(text.r#type, RichTextType::TonAddress);
    let rich = RichMessage::default();
    let dates = DateContext::utc();
    let mut context = HtmlContext::default();
    let mut media = NativeRichMedia::new("", "");
    let mut renderer = RichHtmlRenderer::new(
        &mut context,
        &rich,
        7,
        "https://t.me/",
        "",
        &mut media,
        &dates,
    );
    assert_eq!(
        renderer.render_text(&text).unwrap(),
        "<span class=\"rich_ton_address\">EQ&lt;address&gt;</span>"
    );
    let mut context = JsonContext::default();
    let mut context = RichSerializeContext {
        json: &mut context,
        message: &rich,
        dates: &dates,
    };
    let value: Value =
        serde_json::from_slice(&json_rich::serialize_rich_text(&mut context, &text).unwrap())
            .unwrap();
    assert_eq!(
        value,
        json!({"type":"ton_address","text":{"type":"plain","text":"EQ<address>"}})
    );
    for source in [
        RichMapPointSource::GeoPoint,
        RichMapPointSource::InputGeoPoint,
    ] {
        let bytes = json_rich::map_point(
            &context,
            &RichMapPoint {
                latitude: f64::NAN,
                longitude: f64::INFINITY,
                source,
                ..Default::default()
            },
        );
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["latitude"].is_null() && value["longitude"].is_null());
    }
    for (lat, lon, valid) in [
        (90.0, 180.0, true),
        (-90.0, -180.0, true),
        (90.0001, 0.0, false),
        (0.0, 180.0001, false),
        (-91.0, 0.0, false),
    ] {
        let point = parse_geo_point(&json!({"_":"GeoPoint","lat":lat,"long":lon}));
        assert_eq!(point.valid, valid);
        if !valid {
            assert_eq!((point.latitude, point.longitude), (0.0, 0.0));
        }
    }
    assert!(!parse_geo_point(&json!({"_":"GeoPoint","lat":null,"long":0})).valid);
}
pub fn html_safety() {
    let input = "<script>&\"'";
    let escaped = "&lt;script&gt;&amp;&quot;&apos;";
    let (html, _) = service(
        Action::CustomAction(ActionCustomAction {
            message: input.into(),
        }),
        false,
        0,
    );
    assert_eq!(html, escaped);
    let (html, _) = service(
        Action::SetChatTheme(ActionSetChatTheme {
            emoji: input.into(),
        }),
        false,
        0,
    );
    assert_eq!(html, format!("Deleted changed chat theme to {escaped}"));
    let (html, _) = service(
        Action::PollDeleteAnswer(ActionPollDeleteAnswer {
            option: input.into(),
        }),
        false,
        0,
    );
    assert_eq!(
        html,
        format!("Deleted removed &quot;{escaped}&quot; from the poll.")
    );
    let (html, _) = service(
        Action::PollAppendAnswer(ActionPollAppendAnswer {
            option: input.into(),
        }),
        false,
        0,
    );
    assert_eq!(
        html,
        format!("Deleted added &quot;{escaped}&quot; to the poll.")
    );
    let (html, _) = service(
        Action::GiftPremium(ActionGiftPremium {
            cost: input.into(),
            days: 30,
        }),
        false,
        0,
    );
    assert_eq!(
        html,
        format!("Deleted sent you a gift for {escaped}: Telegram Premium for 30 days.")
    );
    let (html, _) = service(
        Action::GiftCredits(ActionGiftCredits {
            cost: input.into(),
            amount: CreditsAmount::make(5, 0, CreditsType::Stars),
        }),
        false,
        0,
    );
    assert_eq!(
        html,
        format!("Deleted sent you a gift for {escaped}: 5 Telegram Stars.")
    );
    let link = html_text::format_custom_emoji("x\" onload=\"bad", "x", "../a&");
    assert_eq!(
        link,
        "<a href = \"../a&amp;x&quot; onload=&quot;bad\">x</a>"
    );
    for target in ["javascript:alert(1)", "data:text/html,test", "vbscript:bad"] {
        let message = Message {
            inline_button_rows: vec![vec![HistoryMessageMarkupButton {
                r#type: MarkupButtonType::Url,
                text: "public".into(),
                data: target.as_bytes().to_vec(),
                ..Default::default()
            }]],
            ..Default::default()
        };
        let html = WrapBase::default().push_inline_buttons(&message).unwrap();
        assert!(!html.contains("href="));
        assert!(!html.contains(target));
    }
}
