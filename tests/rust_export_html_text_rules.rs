#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::{
    html_text::*,
    model::peer_from_user,
    model_peers::{ContactInfo, Peer, PeerUser},
};
#[test]
fn byte_mid_cuts_utf8_bytes() {
    assert_eq!(byte_mid("@user", 1), b"user");
    assert_eq!(byte_mid("éx", 1), [0xa9, b'x']);
}
#[test]
fn context_indents_block_tags_and_sorts_attributes() {
    let mut context = HtmlContext::default();
    assert_eq!(
        context.push_tag("div", &attrs([("id", "m1"), ("class", "a")])),
        "\n<div class=\"a\" id=\"m1\">\n"
    );
    assert_eq!(
        context.push_tag("div", &attrs([("class", "b")])),
        "\n <div class=\"b\">\n"
    );
    assert_eq!(
        context.push_tag("span", &attrs([("inline", ""), ("title", "x\"y")])),
        "<span title=\"x&quot;y\">"
    );
    assert_eq!(
        context.push_tag(
            "img",
            &attrs([("src", "p.jpg"), ("class", "c"), ("empty", "")])
        ),
        "\n   <img class=\"c\" src=\"p.jpg\"/>\n"
    );
    assert_eq!(context.pop_tag().unwrap(), "</span>");
    assert_eq!(context.pop_tag().unwrap(), "\n </div>\n");
    assert_eq!(context.pop_tag().unwrap(), "\n</div>\n");
    assert!(context.empty());
}
#[test]
fn peers_map_names() {
    let mut peers = PeersMap::default();
    peers.data.insert(
        peer_from_user(5),
        Peer::User(PeerUser {
            bare_id: 5,
            info: ContactInfo {
                user_id: 5,
                first_name: "A<nn".into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    );
    assert_eq!(peers.wrap_peer_name(peer_from_user(5)), "A&lt;nn");
    assert_eq!(peers.wrap_peer_name(peer_from_user(6)), "Deleted");
    assert_eq!(
        peers.wrap_user_names(&[5, 6, 7]),
        "A&lt;nn, Deleted Account and Deleted Account"
    );
    let mut names = UserpicData::default();
    fill_userpic_names_from_full(&mut names, "First  Middle Last");
    assert_eq!(
        (&*names.first_name, &*names.last_name),
        ("First", "Middle Last")
    );
}
#[test]
fn push_userpic_initials_and_images() {
    let mut wrap = WrapBase::new("../");
    assert_eq!(
        wrap.push_userpic(&UserpicData {
            color_index: 2,
            pixel_size: 42,
            first_name: " Ann".into(),
            last_name: "Lee".into(),
            ..Default::default()
        })
        .unwrap(),
        r###"
<div class="userpic userpic3" style="width: 42px; height: 42px">

 <div class="initials" style="line-height: 42px">
AL
 </div>

</div>
"###
    );
    assert_eq!(
        wrap.push_userpic(&UserpicData {
            pixel_size: 20,
            first_name: "😀x".into(),
            tooltip: "T".into(),
            ..Default::default()
        })
        .unwrap(),
        r###"
<div class="userpic userpic1" style="width: 20px; height: 20px">

 <div class="initials" style="line-height: 20px" title="T">
?
 </div>

</div>
"###
    );
    assert_eq!(
        wrap.push_userpic(&UserpicData {
            pixel_size: 60,
            image_link: "p_thumb.jpg".into(),
            large_link: "p.jpg".into(),
            ..Default::default()
        })
        .unwrap(),
        r###"
<a class="userpic_link" href="../p.jpg">

 <img class="userpic" src="../p_thumb.jpg" style="width: 60px; height: 60px"/>

</a>
"###
    );
}
