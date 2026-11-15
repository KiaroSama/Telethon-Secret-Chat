#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{Deserializable, Serializable, enums, wire};
fn check(id: u64) {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/rust-schema-oracle.json")).unwrap();
    let r = data["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"].as_u64() == Some(id))
        .unwrap();
    let bytes = hex::decode(r["hex"].as_str().unwrap()).unwrap();
    let family = r["family"].as_str().unwrap();
    wire::validate(&bytes, family).unwrap();
    let encoded = match family {
        "DecryptedMessage" => enums::DecryptedMessage::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DecryptedMessageAction" => enums::DecryptedMessageAction::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DecryptedMessageLayer" => enums::DecryptedMessageLayer::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DecryptedMessageMedia" => enums::DecryptedMessageMedia::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "DocumentAttribute" => enums::DocumentAttribute::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "FileLocation" => enums::FileLocation::from_bytes(&bytes).unwrap().to_bytes(),
        "GroupCallMessage" => enums::GroupCallMessage::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "InputStickerSet" => enums::InputStickerSet::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "JSONObjectValue" => enums::JsonobjectValue::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "JSONValue" => enums::Jsonvalue::from_bytes(&bytes).unwrap().to_bytes(),
        "MessageEntity" => enums::MessageEntity::from_bytes(&bytes).unwrap().to_bytes(),
        "PhotoSize" => enums::PhotoSize::from_bytes(&bytes).unwrap().to_bytes(),
        "SendMessageAction" => enums::SendMessageAction::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        "TextWithEntities" => enums::TextWithEntities::from_bytes(&bytes)
            .unwrap()
            .to_bytes(),
        _ => panic!("Unknown oracle family"),
    };
    assert_eq!(encoded, bytes);
}
#[test]
fn constructor_decryptedmessage_1f814f1f() {
    check(528568095);
}
#[test]
fn constructor_decryptedmessageservice_aa48327d() {
    check(2856858237);
}
#[test]
fn constructor_decryptedmessagemediaempty_089f5c4a() {
    check(144661578);
}
#[test]
fn constructor_decryptedmessagemediaphoto_32798a8c() {
    check(846826124);
}
#[test]
fn constructor_decryptedmessagemediavideo_4cee6ef3() {
    check(1290694387);
}
#[test]
fn constructor_decryptedmessagemediageopoint_35480a59() {
    check(893913689);
}
#[test]
fn constructor_decryptedmessagemediacontact_588a0a97() {
    check(1485441687);
}
#[test]
fn constructor_decryptedmessageactionsetmessagettl_a1733aec() {
    check(2708683500);
}
#[test]
fn constructor_decryptedmessagemediadocument_b095434b() {
    check(2962572107);
}
#[test]
fn constructor_decryptedmessagemediaaudio_6080758f() {
    check(1619031439);
}
#[test]
fn constructor_decryptedmessageactionreadmessages_0c4f40be() {
    check(206520510);
}
#[test]
fn constructor_decryptedmessageactiondeletemessages_65614304() {
    check(1700872964);
}
#[test]
fn constructor_decryptedmessageactionscreenshotmessages_8ac1f475() {
    check(2327966837);
}
#[test]
fn constructor_decryptedmessageactionflushhistory_6719e45c() {
    check(1729750108);
}
#[test]
fn constructor_decryptedmessage_204d3878() {
    check(541931640);
}
#[test]
fn constructor_decryptedmessageservice_73164160() {
    check(1930838368);
}
#[test]
fn constructor_decryptedmessagemediavideo_524a415d() {
    check(1380598109);
}
#[test]
fn constructor_decryptedmessagemediaaudio_57e0a9cb() {
    check(1474341323);
}
#[test]
fn constructor_decryptedmessagelayer_1be31789() {
    check(467867529);
}
#[test]
fn constructor_sendmessagetypingaction_16bf744e() {
    check(381645902);
}
#[test]
fn constructor_sendmessagecancelaction_fd5ec8f5() {
    check(4250847477);
}
#[test]
fn constructor_sendmessagerecordvideoaction_a187d66f() {
    check(2710034031);
}
#[test]
fn constructor_sendmessageuploadvideoaction_92042ff7() {
    check(2449747959);
}
#[test]
fn constructor_sendmessagerecordaudioaction_d52f73f7() {
    check(3576656887);
}
#[test]
fn constructor_sendmessageuploadaudioaction_e6ac8a6f() {
    check(3870067311);
}
#[test]
fn constructor_sendmessageuploadphotoaction_990a3c1a() {
    check(2567584794);
}
#[test]
fn constructor_sendmessageuploaddocumentaction_8faee98e() {
    check(2410604942);
}
#[test]
fn constructor_sendmessagegeolocationaction_176f8ba1() {
    check(393186209);
}
#[test]
fn constructor_sendmessagechoosecontactaction_628cbc6f() {
    check(1653390447);
}
#[test]
fn constructor_decryptedmessageactionresend_511110b0() {
    check(1360072880);
}
#[test]
fn constructor_decryptedmessageactionnotifylayer_f3048883() {
    check(4077160579);
}
#[test]
fn constructor_decryptedmessageactiontyping_ccb27641() {
    check(3434247745);
}
#[test]
fn constructor_decryptedmessageactionrequestkey_f3c9611b() {
    check(4090061083);
}
#[test]
fn constructor_decryptedmessageactionacceptkey_6fe1735b() {
    check(1877046107);
}
#[test]
fn constructor_decryptedmessageactionabortkey_dd05ec6b() {
    check(3708152939);
}
#[test]
fn constructor_decryptedmessageactioncommitkey_ec2e0b9b() {
    check(3962440603);
}
#[test]
fn constructor_decryptedmessageactionnoop_a82fdd63() {
    check(2821709155);
}
#[test]
fn constructor_documentattributeimagesize_6c37c15c() {
    check(1815593308);
}
#[test]
fn constructor_documentattributeanimated_11b58939() {
    check(297109817);
}
#[test]
fn constructor_documentattributesticker_fb0a5727() {
    check(4211758887);
}
#[test]
fn constructor_documentattributevideo_5910cccb() {
    check(1494273227);
}
#[test]
fn constructor_documentattributeaudio_051448e5() {
    check(85215461);
}
#[test]
fn constructor_documentattributefilename_15590068() {
    check(358154344);
}
#[test]
fn constructor_photosizeempty_0e17e23c() {
    check(236446268);
}
#[test]
fn constructor_photosize_77bfb61b() {
    check(2009052699);
}
#[test]
fn constructor_photocachedsize_e9a734fa() {
    check(3920049402);
}
#[test]
fn constructor_filelocationunavailable_7c596b46() {
    check(2086234950);
}
#[test]
fn constructor_filelocation_53d69076() {
    check(1406570614);
}
#[test]
fn constructor_decryptedmessagemediaexternaldocument_fa95b0dd() {
    check(4204114141);
}
#[test]
fn constructor_decryptedmessage_36b091de() {
    check(917541342);
}
#[test]
fn constructor_decryptedmessagemediaphoto_f1fa8d78() {
    check(4059729272);
}
#[test]
fn constructor_decryptedmessagemediavideo_970c8c0e() {
    check(2534181902);
}
#[test]
fn constructor_decryptedmessagemediadocument_7afe8ae2() {
    check(2063502050);
}
#[test]
fn constructor_documentattributesticker_3a556302() {
    check(978674434);
}
#[test]
fn constructor_documentattributeaudio_ded218e0() {
    check(3738310880);
}
#[test]
fn constructor_messageentityunknown_bb92ba95() {
    check(3146955413);
}
#[test]
fn constructor_messageentitymention_fa04579d() {
    check(4194588573);
}
#[test]
fn constructor_messageentityhashtag_6f635b0d() {
    check(1868782349);
}
#[test]
fn constructor_messageentitybotcommand_6cef8ac7() {
    check(1827637959);
}
#[test]
fn constructor_messageentityurl_6ed02538() {
    check(1859134776);
}
#[test]
fn constructor_messageentityemail_64e475c2() {
    check(1692693954);
}
#[test]
fn constructor_messageentitybold_bd610bc9() {
    check(3177253833);
}
#[test]
fn constructor_messageentityitalic_826f8b60() {
    check(2188348256);
}
#[test]
fn constructor_messageentitycode_28a20571() {
    check(681706865);
}
#[test]
fn constructor_messageentitypre_73924be0() {
    check(1938967520);
}
#[test]
fn constructor_messageentitytexturl_76a6d327() {
    check(1990644519);
}
#[test]
fn constructor_inputstickersetshortname_861cc8a0() {
    check(2250033312);
}
#[test]
fn constructor_inputstickersetempty_ffb62b95() {
    check(4290128789);
}
#[test]
fn constructor_decryptedmessagemediavenue_8a0df56f() {
    check(2316170607);
}
#[test]
fn constructor_decryptedmessagemediawebpage_e50511d8() {
    check(3842314712);
}
#[test]
fn constructor_documentattributeaudio_9852f9c6() {
    check(2555574726);
}
#[test]
fn constructor_documentattributevideo_0ef02ce6() {
    check(250621158);
}
#[test]
fn constructor_sendmessagerecordroundaction_88f27fbc() {
    check(2297593788);
}
#[test]
fn constructor_sendmessageuploadroundaction_bb718624() {
    check(3144779300);
}
#[test]
fn constructor_decryptedmessage_91cc4674() {
    check(2446083700);
}
#[test]
fn constructor_messageentityunderline_9c4e7e8b() {
    check(2622389899);
}
#[test]
fn constructor_messageentitystrike_bf0693d4() {
    check(3204879316);
}
#[test]
fn constructor_messageentityblockquote_020df5d0() {
    check(34469328);
}
#[test]
fn constructor_decryptedmessagemediadocument_6abd9782() {
    check(1790809986);
}
#[test]
fn constructor_messageentityspoiler_32ca960f() {
    check(852137487);
}
#[test]
fn constructor_messageentitycustomemoji_c8cf05f8() {
    check(3369010680);
}
#[test]
fn constructor_jsonobjectvalue_c0de1bd9() {
    check(3235781593);
}
#[test]
fn constructor_jsonnull_3f6d7b68() {
    check(1064139624);
}
#[test]
fn constructor_jsonbool_c7345e6a() {
    check(3342098026);
}
#[test]
fn constructor_jsonnumber_2be0dfa4() {
    check(736157604);
}
#[test]
fn constructor_jsonstring_b71e767a() {
    check(3072226938);
}
#[test]
fn constructor_jsonarray_f7444763() {
    check(4148447075);
}
#[test]
fn constructor_jsonobject_99c1d49d() {
    check(2579616925);
}
#[test]
fn constructor_textwithentities_751f3146() {
    check(1964978502);
}
#[test]
fn constructor_groupcallmessage_907ce88e() {
    check(2424105102);
}
