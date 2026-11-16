#![allow(clippy::unwrap_used)]
//! Complete rich HTML output, geometry and media side-effect scenarios.
use image::GenericImageView;
use kiaro_secret_chat::export::{
    error::Result,
    format::DateContext,
    html_layout,
    html_rich::{RichMediaCallbacks, render_rich_message},
    html_rich_media::NativeRichMedia,
    html_rich_support::MediaData,
    html_text::{HtmlContext, WrapBase},
    model::{Document, Photo},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
fn decode<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}
fn encode<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap()
}
struct Stubs;
fn photo_id(p: Option<&Photo>) -> String {
    p.map_or_else(|| "None".into(), |p| p.id.to_string())
}
fn document_id(d: Option<&Document>) -> String {
    d.map_or_else(|| "None".into(), |d| d.id.to_string())
}
fn card(d: &MediaData) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        d.title, d.description, d.status, d.classes, d.link
    )
}
impl RichMediaCallbacks for Stubs {
    fn photo(&mut self, _: &mut HtmlContext, p: Option<&Photo>) -> Result<String> {
        Ok(format!("[photo {}]", photo_id(p)))
    }
    fn video(&mut self, _: &mut HtmlContext, d: Option<&Document>) -> Result<String> {
        Ok(format!("[video {}]", document_id(d)))
    }
    fn audio(&mut self, _: &mut HtmlContext, d: Option<&Document>) -> Result<String> {
        Ok(format!("[audio {}]", document_id(d)))
    }
    fn file(&mut self, _: &mut HtmlContext, d: Option<&Document>) -> Result<String> {
        Ok(format!("[file {}]", document_id(d)))
    }
    fn generic(&mut self, _: &mut HtmlContext, d: MediaData) -> Result<String> {
        Ok(format!("[generic {}]", card(&d)))
    }
    fn photo_card(
        &mut self,
        _: &mut HtmlContext,
        d: MediaData,
        p: Option<&Photo>,
    ) -> Result<String> {
        Ok(format!("[card {}|{}]", card(&d), photo_id(p)))
    }
}
fn files(root: &Path, folder: &Path, out: &mut BTreeMap<String, Value>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(root, &path, out);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|p| p.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            let size = fs::read(&path)
                .ok()
                .and_then(|b| image::load_from_memory(&b).ok())
                .map(|i| i.dimensions());
            out.insert(name, json!({"size":size}));
        }
    }
}
fn run(name: &str) {
    let case: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tdexport_rich")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap();
    let traces = case["traces"].as_array().unwrap();
    assert!(!traces.is_empty());
    for trace in traces {
        let args = trace["args"].as_array().unwrap();
        let mut after = args.clone();
        let function = trace["function"].as_str().unwrap();
        let dates = DateContext::named(trace["timezone"].as_str().unwrap_or("UTC")).unwrap();
        let folder = tempfile::tempdir().unwrap();
        if let Some(before) = trace["files_before"].as_object() {
            for (name, data) in before {
                let path = folder.path().join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, decode::<Vec<u8>>(data)).unwrap();
            }
        }
        let result = match function {
            "html_rich.render_rich_message" => {
                let mut context: HtmlContext = decode(&args[0]);
                let message: kiaro_secret_chat::export::model_rich::RichMessage = decode(&args[1]);
                if name == "a130b09b815b4ef1.json" {
                    let mut extreme = message.clone();
                    for (photo, (width, height)) in extreme
                        .photos
                        .values_mut()
                        .zip([(u32::MAX, 1), (1, u32::MAX)])
                    {
                        photo.image.width = width;
                        photo.image.height = height;
                    }
                    let html = render_rich_message(
                        &mut HtmlContext::default(),
                        &extreme,
                        5,
                        "https://t.me/",
                        "",
                        &mut Stubs,
                        &dates,
                    )
                    .unwrap();
                    assert!(html.contains("aspect-ratio: 4294967295 / 1"));
                }
                let mut stubs = Stubs;
                let mut native =
                    NativeRichMedia::new(trace["base"].as_str().unwrap(), folder.path());
                let callbacks: &mut dyn RichMediaCallbacks = if trace["callbacks"] == "stubs" {
                    &mut stubs
                } else {
                    &mut native
                };
                let output = render_rich_message(
                    &mut context,
                    &message,
                    args[2].as_i64().unwrap(),
                    args[3].as_str().unwrap(),
                    args[4].as_str().unwrap(),
                    callbacks,
                    &dates,
                )
                .unwrap();
                after[0] = encode(context);
                encode(output)
            }
            "html_layout.layout_media_group_geometry" => encode(
                html_layout::layout_media_group_geometry(
                    &decode::<Vec<_>>(&args[0]),
                    args[1].as_i64().unwrap(),
                    args[2].as_i64().unwrap(),
                    args[3].as_i64().unwrap(),
                )
                .unwrap(),
            ),
            "html_layout.united" => encode(html_layout::united(decode(&args[0]), decode(&args[1]))),
            _ => {
                let mut wrap = WrapBase {
                    base: args[0]["base"].as_str().unwrap().into(),
                    context: decode(&args[0]["context"]),
                };
                let output = match function {
                    "html_media.MediaMixin.push_photo_media" => {
                        wrap.push_photo_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_sticker_media" => {
                        wrap.push_sticker_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_video_file_media" => {
                        wrap.push_video_file_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_animated_media" => {
                        wrap.push_animated_media(&decode(&args[1]), folder.path())
                    }
                    _ => panic!("Unregistered native rich function: {function}"),
                }
                .unwrap();
                after[0] = json!({"base":wrap.base,"context":wrap.context});
                encode(output)
            }
        };
        assert_eq!(
            result,
            trace
                .get("current_result")
                .unwrap_or(&trace["result"])
                .clone(),
            "{}: {function} output",
            case["legacy_case"]
        );
        if trace.get("after").is_some() {
            assert_eq!(
                encode(after),
                trace["after"],
                "{}: {function} mutation",
                case["legacy_case"]
            );
        }
        if let Some(before) = trace["files_before"].as_object() {
            for (name, data) in before {
                assert_eq!(
                    fs::read(folder.path().join(name)).unwrap(),
                    decode::<Vec<u8>>(data),
                    "Source media changed"
                );
            }
            let mut actual = BTreeMap::new();
            files(folder.path(), folder.path(), &mut actual);
            assert_eq!(
                encode(actual),
                trace["files_after"],
                "{}: {function} filesystem",
                case["legacy_case"]
            );
        }
    }
}
#[test]
fn rich_88a34b68df415e91() {
    run("88a34b68df415e91.json");
}
#[test]
fn rich_346390a17addb029() {
    run("346390a17addb029.json");
}
#[test]
fn rich_3ee4155ada09dcc4() {
    run("3ee4155ada09dcc4.json");
}
#[test]
fn rich_e40c4b1c1d5ee73b() {
    run("e40c4b1c1d5ee73b.json");
}
#[test]
fn rich_df193040630581e4() {
    run("df193040630581e4.json");
}
#[test]
fn rich_0529933bfe27da0a() {
    run("0529933bfe27da0a.json");
}
#[test]
fn rich_ade5e3f5433dd3ab() {
    run("ade5e3f5433dd3ab.json");
}
#[test]
fn rich_132b0c129bb27820() {
    run("132b0c129bb27820.json");
}
#[test]
fn rich_de4b76e703839467() {
    run("de4b76e703839467.json");
}
#[test]
fn rich_c1f1798b08394e49() {
    run("c1f1798b08394e49.json");
}
#[test]
fn rich_f6359b3c72c9dc77() {
    run("f6359b3c72c9dc77.json");
}
#[test]
fn rich_a00208feac97116d() {
    run("a00208feac97116d.json");
}
#[test]
fn rich_723c5a54aa0211a4() {
    run("723c5a54aa0211a4.json");
}
#[test]
fn rich_c37b4a288a52061e() {
    run("c37b4a288a52061e.json");
}
#[test]
fn rich_da99cd794cc6882d() {
    run("da99cd794cc6882d.json");
}
#[test]
fn rich_df466f85aeb0527d() {
    run("df466f85aeb0527d.json");
}
#[test]
fn rich_9cee40ac24bceb8e() {
    run("9cee40ac24bceb8e.json");
}
#[test]
fn rich_1380afeae4b05dca() {
    use kiaro_secret_chat::export::{model::Image, model_rich::*};
    let mut message = RichMessage::default();
    let items = (1..=13)
        .map(|id| {
            message.photos.insert(
                id,
                Photo {
                    id,
                    image: Image {
                        width: 16,
                        height: 9,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            RichBlock {
                kind: RichBlockKind::Photo,
                photo_id: id,
                ..Default::default()
            }
        })
        .collect();
    message.blocks.push(RichBlock {
        kind: RichBlockKind::Collage,
        blocks: items,
        ..Default::default()
    });
    let output = render_rich_message(
        &mut HtmlContext::default(),
        &message,
        1,
        "https://t.me/",
        "",
        &mut Stubs,
        &DateContext::utc(),
    )
    .unwrap();
    assert_eq!(output.matches("[photo ").count(), 13);
    assert!(output.contains("rich_media_group_items"));

    assert!(
        html_layout::layout_media_group_geometry(&[(16, 9); 13], 430, 100, 4)
            .unwrap()
            .is_empty()
    );
    run("1380afeae4b05dca.json");
}
#[test]
fn rich_2752bf0caaef9924() {
    run("2752bf0caaef9924.json");
}
#[test]
fn rich_dbcf53e323cf55f9() {
    run("dbcf53e323cf55f9.json");
}
#[test]
fn rich_d7724de3225aac3d() {
    run("d7724de3225aac3d.json");
}
#[test]
fn rich_e7674847c2b683b1() {
    run("e7674847c2b683b1.json");
}
#[test]
fn rich_820bcc348802b3a2() {
    run("820bcc348802b3a2.json");
}
#[test]
fn rich_4d4bbf810fcaf39e() {
    run("4d4bbf810fcaf39e.json");
}
#[test]
fn rich_ec5181f0b9715d75() {
    run("ec5181f0b9715d75.json");
}
#[test]
fn rich_afe850fe52160f6c() {
    run("afe850fe52160f6c.json");
}
#[test]
fn rich_6f7a1eb633f8e720() {
    run("6f7a1eb633f8e720.json");
}
#[test]
fn rich_86abb05c4f18b2bc() {
    run("86abb05c4f18b2bc.json");
}
#[test]
fn rich_a130b09b815b4ef1() {
    run("a130b09b815b4ef1.json");
}
#[test]
fn rich_309c2e0a434946e8() {
    run("309c2e0a434946e8.json");
}
#[test]
fn rich_2e7b8c3bbbb12d88() {
    run("2e7b8c3bbbb12d88.json");
}
#[test]
fn rich_16bdd18cdd723237() {
    run("16bdd18cdd723237.json");
}
#[test]
fn rich_ca1c343901c0d209() {
    run("ca1c343901c0d209.json");
}
#[test]
fn rich_8f4b17e830e6bfe0() {
    run("8f4b17e830e6bfe0.json");
}
#[test]
fn rich_fe0ca4e2c407ab59() {
    run("fe0ca4e2c407ab59.json");
}
#[test]
fn rich_4224a4daafd60e1c() {
    run("4224a4daafd60e1c.json");
}
#[test]
fn rich_ddfe1afbf118faee() {
    run("ddfe1afbf118faee.json");
}
#[test]
fn rich_0b2ce61deb24e3a2() {
    run("0b2ce61deb24e3a2.json");
}
#[test]
fn rich_f84ed446a1bdff07() {
    run("f84ed446a1bdff07.json");
}
#[test]
fn rich_bc84e213d744dfff() {
    run("bc84e213d744dfff.json");
}
#[test]
fn rich_3a801876183a6c69() {
    run("3a801876183a6c69.json");
}
#[test]
fn rich_adbefdef6a4b77e4() {
    run("adbefdef6a4b77e4.json");
}
#[test]
fn rich_2836223826f44038() {
    run("2836223826f44038.json");
}
#[test]
fn rich_cf4abb1f19b07359() {
    run("cf4abb1f19b07359.json");
}
#[test]
fn rich_fe6a427f97c443a3() {
    run("fe6a427f97c443a3.json");
}
#[test]
fn rich_0421aab486f3cf4a() {
    run("0421aab486f3cf4a.json");
}
#[test]
fn rich_d605d9b2cd1c1a8e() {
    run("d605d9b2cd1c1a8e.json");
}
#[test]
fn rich_5e6b6b56b2704743() {
    run("5e6b6b56b2704743.json");
}
#[test]
fn rich_15ca6133652a5bdc() {
    run("15ca6133652a5bdc.json");
}
#[test]
fn rich_c5a411d3889b4314() {
    run("c5a411d3889b4314.json");
}
#[test]
fn rich_7d9ebd8126a8b1e2() {
    run("7d9ebd8126a8b1e2.json");
}
#[test]
fn rich_8cd08e6fe26fa240() {
    run("8cd08e6fe26fa240.json");
}
#[test]
fn rich_9c7864b0b3dcab10() {
    run("9c7864b0b3dcab10.json");
}
#[test]
fn rich_83c7bda1df3efdf0() {
    run("83c7bda1df3efdf0.json");
}
#[test]
fn rich_fefc558dbebd6b50() {
    run("fefc558dbebd6b50.json");
}
#[test]
fn rich_73cb414e9cbdc23e() {
    run("73cb414e9cbdc23e.json");
}
#[test]
fn rich_302dfe801bb0541e() {
    run("302dfe801bb0541e.json");
}
#[test]
fn rich_6ab9ee618f0d46e2() {
    run("6ab9ee618f0d46e2.json");
}
#[test]
fn rich_3e0b00b3516825b7() {
    run("3e0b00b3516825b7.json");
}
#[test]
fn rich_99f165c791db390a() {
    run("99f165c791db390a.json");
}
#[test]
fn rich_0c831b445ebf52ad() {
    run("0c831b445ebf52ad.json");
}
#[test]
fn rich_78e4db5ac7fd91c8() {
    run("78e4db5ac7fd91c8.json");
}
#[test]
fn rich_f8800cb97a0530bf() {
    run("f8800cb97a0530bf.json");
}
#[test]
fn rich_1100d58b5eae19b9() {
    run("1100d58b5eae19b9.json");
}
#[test]
fn rich_a30cacbd4acdba2b() {
    run("a30cacbd4acdba2b.json");
}
#[test]
fn rich_710092257240c3fe() {
    run("710092257240c3fe.json");
}
#[test]
fn rich_8281c854b0aac899() {
    run("8281c854b0aac899.json");
}
#[test]
fn rich_8fe0ad8f39ab971a() {
    run("8fe0ad8f39ab971a.json");
}
#[test]
fn rich_8de65ce74a333b79() {
    run("8de65ce74a333b79.json");
}
#[test]
fn rich_a80131954e92d3fb() {
    run("a80131954e92d3fb.json");
}
#[test]
fn rich_11f10d25db614f07() {
    run("11f10d25db614f07.json");
}
#[test]
fn rich_bc2d841ba8cf763c() {
    run("bc2d841ba8cf763c.json");
}
#[test]
fn rich_cef1bb8955a04e6e() {
    run("cef1bb8955a04e6e.json");
}
#[test]
fn rich_8c29fddaa6070161() {
    run("8c29fddaa6070161.json");
}
#[test]
fn rich_f24b41c7168f9aa0() {
    run("f24b41c7168f9aa0.json");
}
#[test]
fn rich_b5d6dec52b68ef01() {
    run("b5d6dec52b68ef01.json");
}
#[test]
fn rich_615084d9892c7fd8() {
    run("615084d9892c7fd8.json");
}
#[test]
fn rich_852ec1112affddc4() {
    run("852ec1112affddc4.json");
}
#[test]
fn rich_29089b501625d53a() {
    run("29089b501625d53a.json");
}
#[test]
fn rich_7873ace0d861ebcb() {
    run("7873ace0d861ebcb.json");
}
#[test]
fn rich_7d274a1f35e8483a() {
    run("7d274a1f35e8483a.json");
}
#[test]
fn rich_733362ddffb257b5() {
    run("733362ddffb257b5.json");
}
#[test]
fn rich_1aa1eff82c8a383d() {
    run("1aa1eff82c8a383d.json");
}
#[test]
fn rich_92254a4019cf14a3() {
    run("92254a4019cf14a3.json");
}
