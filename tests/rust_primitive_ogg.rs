#![allow(clippy::unwrap_used)]
#[path = "support_primitive/ogg.rs"]
mod ogg;
#[test]
fn ogg_75acb3036c92bb30() {
    ogg::audit("ENCODER=engine TITLE=example");
}
#[test]
fn ogg_12963afe981193f2() {
    ogg::audit("COMMENT= ARTIST=Nobody");
}
#[test]
fn ogg_413429e4e03ca756() {
    ogg::audit("MYALBUM=demo");
}
#[test]
fn ogg_85d73a3b304477eb() {
    ogg::audit("ENCODER= ALBUM=not-a-tag");
}
#[test]
fn ogg_bf9bfa8343b93b3b() {
    ogg::audit("truncate_1");
}
#[test]
fn ogg_1c239b159c0a94ab() {
    ogg::audit("truncate_2");
}
#[test]
fn ogg_ac7ec2d0436c846c() {
    ogg::audit("truncate_5");
}
#[test]
fn ogg_062573d3d84fe599() {
    ogg::audit("truncate_10");
}
#[test]
fn ogg_0b035aba022daea8() {
    ogg::audit("lowercase");
}
#[test]
fn ogg_a8646c9fec84f092() {
    ogg::audit("split_music_opus");
}
#[test]
fn ogg_48e5e6700adf90fa() {
    ogg::audit("split_music_vorbis");
}
#[test]
fn ogg_f78a24c34d0c5c8f() {
    ogg::audit("split_voice_opus");
}
#[test]
fn ogg_1eb2913f487317c6() {
    ogg::audit("split_voice_vorbis");
}
#[test]
fn ogg_3b87c7e92404ea53() {
    ogg::audit("magic_outside_comment");
}
#[test]
fn ogg_83da39c12fd54299() {
    ogg::audit("other_logical_stream");
}
#[test]
fn ogg_8d2a7a29173737ca() {
    ogg::audit("bad capture pattern");
}
#[test]
fn ogg_1a03d3bd6c38990b() {
    ogg::audit("unknown stream structure version");
}
#[test]
fn ogg_7bacba2ed055fc2b() {
    ogg::audit("first page is not beginning of stream");
}
#[test]
fn ogg_c2faa799a2339ddb() {
    ogg::audit("lacing runs past the buffer");
}
#[test]
fn ogg_5bd4d6e5e15aa22e() {
    ogg::audit("packet unfinished at buffer end");
}
#[test]
fn ogg_cc25272a91073a4a() {
    ogg::audit("continuation flag with nothing to continue");
}
#[test]
fn ogg_6ca2615b9a27bbdf() {
    ogg::audit("first packet is not an identification header");
}
#[test]
fn ogg_d5aa90c3fe618978() {
    ogg::voice("recording");
}
#[test]
fn ogg_d2f01937d76b823c() {
    ogg::voice("encoder");
}
#[test]
fn ogg_3d884f0f07e21874() {
    ogg::voice("TITLE=A Song");
}
#[test]
fn ogg_3924751639498a4a() {
    ogg::voice("ARTIST=Someone");
}
#[test]
fn ogg_6931a5086d548061() {
    ogg::voice("ALBUM=A Record");
}
#[test]
fn ogg_e4011d9cadcd86ce() {
    ogg::voice("TRACKNUMBER=3");
}
#[test]
fn ogg_2809043b0d16d8d6() {
    ogg::voice("not_ogg");
}
#[test]
fn ogg_d9ae04378afac55a() {
    ogg::voice("tag_inside_value");
}
#[test]
fn ogg_a4374b05469c273e() {
    ogg::voice("infer_voice");
}
#[test]
fn ogg_be8e95d061e6c1f8() {
    ogg::voice("infer_music");
}
#[test]
fn ogg_eb8674d041f05fcf() {
    ogg::voice("explicit_voice");
}
#[test]
fn ogg_02adae7ba784417c() {
    ogg::voice("non_audio");
}
#[test]
fn ogg_8bbcfcf1b9f70dd8() {
    ogg::voice("audio/mpeg");
}
#[test]
fn ogg_c928db1a1fb1190a() {
    ogg::voice("audio/mp4");
}
#[test]
fn ogg_42a3102bd6283144() {
    ogg::voice("audio/flac");
}
#[test]
fn ogg_46043775f573218d() {
    ogg::voice("audio/wav");
}
#[test]
fn ogg_9367146647aa2fab() {
    ogg::voice("audio/aac");
}
#[test]
fn ogg_a4410a6d604691be() {
    ogg::voice("audio/ogg");
}
#[test]
fn ogg_a8f3e953b7a693a6() {
    ogg::voice("audio/opus");
}
