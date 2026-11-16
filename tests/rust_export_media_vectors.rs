#![allow(clippy::unwrap_used)]
//! Every media-card scenario, with filesystem and source-preservation checks.
use image::GenericImageView;
use kiaro_secret_chat::export::{
    format::DateContext,
    html_media, html_media_extra,
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
            let size = image::open(&path).ok().map(|image| image.dimensions());
            out.insert(name, json!({"size":size}));
        }
    }
}
fn run(name: &str) {
    let case: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tdexport_media")
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
            "html_media.prepare_media_data" => encode(
                html_media::prepare_media_data(
                    &decode(&args[0]),
                    &decode(&args[1]),
                    args[2].as_str().unwrap(),
                )
                .unwrap(),
            ),
            "html_media.calculate_thumb_size" => {
                let number = |i: usize| args[i].as_u64().unwrap() as u32;
                let retina = args.get(4).and_then(Value::as_bool).unwrap_or(false);
                encode(html_media::calculate_thumb_size(
                    number(0),
                    number(1),
                    number(2),
                    number(3),
                    retina,
                )(decode(&trace["size"])))
            }
            "html_media_extra._participants" => encode(html_media_extra::participants(
                &decode(&args[0]),
                args[1].as_bool().unwrap(),
                args[2].as_bool().unwrap(),
                args[3].as_u64().unwrap() as usize,
            )),
            _ => {
                let mut wrap = WrapBase {
                    base: args[0]["base"].as_str().unwrap().into(),
                    context: decode::<HtmlContext>(&args[0]["context"]),
                };
                let output = match function {
                    "html_media.MediaMixin.push_generic_media" => {
                        wrap.push_generic_media(&decode(&args[1]))
                    }
                    "html_media.MediaMixin.push_sticker_media" => {
                        wrap.push_sticker_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_animated_media" => {
                        wrap.push_animated_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_video_file_media" => {
                        wrap.push_video_file_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_photo_media" => {
                        wrap.push_photo_media(&decode(&args[1]), folder.path())
                    }
                    "html_media.MediaMixin.push_rich_video_media" => wrap.push_rich_video_media(
                        decode::<Option<Document>>(&args[1]).as_ref(),
                        folder.path(),
                    ),
                    "html_media.MediaMixin.push_rich_audio_media" => {
                        wrap.push_rich_audio_media(decode::<Option<Document>>(&args[1]).as_ref())
                    }
                    "html_media.MediaMixin.push_rich_file_media" => {
                        wrap.push_rich_file_media(decode::<Option<Document>>(&args[1]).as_ref())
                    }
                    "html_media.MediaMixin.push_rich_reference_media" => {
                        let mut data: MediaData = decode(&args[1]);
                        let output = wrap.push_rich_reference_media(
                            &mut data,
                            decode::<Option<Photo>>(&args[2]).as_ref(),
                            folder.path(),
                        );
                        after[1] = encode(data);
                        output
                    }
                    "html_media_extra.ExtraMediaMixin.push_giveaway_start" => {
                        wrap.push_giveaway_start(&decode(&args[1]), &decode(&args[2]), &dates)
                    }
                    "html_media_extra.ExtraMediaMixin.push_giveaway_results" => wrap
                        .push_giveaway_results(&decode(&args[1]), &decode(&args[2]), |id, text| {
                            format!("[{id}:{text}]")
                        }),
                    _ => panic!("Unregistered native media function: {function}"),
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
                    "Existing media changed"
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
fn media_1daecd0d835920db() {
    run("1daecd0d835920db.json");
}
#[test]
fn media_58fa173b5df93e65() {
    run("58fa173b5df93e65.json");
}
#[test]
fn media_a0d69dc9659f9c3f() {
    run("a0d69dc9659f9c3f.json");
}
#[test]
fn media_6efa2b894c72b9f9() {
    run("6efa2b894c72b9f9.json");
}
#[test]
fn media_daac80b30d031d6d() {
    run("daac80b30d031d6d.json");
}
#[test]
fn media_3ec42f2ed1744e5d() {
    run("3ec42f2ed1744e5d.json");
}
#[test]
fn media_0e8a2b645b498a78() {
    run("0e8a2b645b498a78.json");
}
#[test]
fn media_8983766c2bdbd415() {
    run("8983766c2bdbd415.json");
}
#[test]
fn media_ea77414c9551084b() {
    run("ea77414c9551084b.json");
}
#[test]
fn media_c4f56701395690fd() {
    run("c4f56701395690fd.json");
}
#[test]
fn media_fe45a1bcdd06589c() {
    run("fe45a1bcdd06589c.json");
}
#[test]
fn media_aef8f4b36bad90aa() {
    run("aef8f4b36bad90aa.json");
}
#[test]
fn media_57547c4e3f3ebeb7() {
    run("57547c4e3f3ebeb7.json");
}
#[test]
fn media_f0390d98eb08ee78() {
    run("f0390d98eb08ee78.json");
}
#[test]
fn media_599eee8d9cf64b22() {
    run("599eee8d9cf64b22.json");
}
#[test]
fn media_8ff15055954183cc() {
    run("8ff15055954183cc.json");
}
#[test]
fn media_5b19d95282893000() {
    run("5b19d95282893000.json");
}
#[test]
fn media_1b3c0fb1890ee98f() {
    run("1b3c0fb1890ee98f.json");
}
#[test]
fn media_d57c87f39c68d83c() {
    run("d57c87f39c68d83c.json");
}
#[test]
fn media_746ce3037abfd820() {
    run("746ce3037abfd820.json");
}
#[test]
fn media_bbd5020a4ca6eca8() {
    run("bbd5020a4ca6eca8.json");
}
#[test]
fn media_533bfbcd65422605() {
    run("533bfbcd65422605.json");
}
#[test]
fn media_45cadb1f28535b78() {
    run("45cadb1f28535b78.json");
}
#[test]
fn media_6323cd9c4da495fe() {
    run("6323cd9c4da495fe.json");
}
#[test]
fn media_d61dcce90270627b() {
    run("d61dcce90270627b.json");
}
#[test]
fn media_50f6c2a1e9819d98() {
    run("50f6c2a1e9819d98.json");
}
#[test]
fn media_e113e8c0947f3b07() {
    run("e113e8c0947f3b07.json");
}
#[test]
fn media_83a1ac62f5d5c2c7() {
    run("83a1ac62f5d5c2c7.json");
}
#[test]
fn media_9302ab8cde748035() {
    run("9302ab8cde748035.json");
}
#[test]
fn media_d705170694125637() {
    run("d705170694125637.json");
}
#[test]
fn media_90494b6baf03e29a() {
    run("90494b6baf03e29a.json");
}
#[test]
fn media_3bea212045d1339f() {
    run("3bea212045d1339f.json");
}
#[test]
fn media_92ba653851fca0aa() {
    run("92ba653851fca0aa.json");
}
#[test]
fn media_0c1aa54d5ce2e5d9() {
    run("0c1aa54d5ce2e5d9.json");
}
#[test]
fn media_fab5caa27daf5493() {
    run("fab5caa27daf5493.json");
}
#[test]
fn media_01738efd09c4655c() {
    run("01738efd09c4655c.json");
}
#[test]
fn media_7817adc1866f5a00() {
    run("7817adc1866f5a00.json");
}
#[test]
fn media_d1bc212f8a8df906() {
    run("d1bc212f8a8df906.json");
}
#[test]
fn media_00d1130f4dc71f41() {
    run("00d1130f4dc71f41.json");
}
