//! Telegram Desktop export css/style.css static data, GPL-3.0.
pub const DATA: &str = r########".default .rich_message .rich_slide .video_file {
    width: 100% !important;
    height: 100% !important;
    object-fit: contain;
    border-radius: 0;
}
.default .rich_message .rich_slideshow_prev,
.default .rich_message .rich_slideshow_next {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 36px;
    height: 36px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: #ffffff;
    color: #333333;
    font-size: 16px;
    line-height: 36px;
    text-align: center;
    opacity: 0.9;
    cursor: pointer;
    z-index: 3;
}
.default .rich_message .rich_slideshow_prev {
    left: 12px;
}
.default .rich_message .rich_slideshow_next {
    right: 12px;
}
.default .rich_message .rich_slideshow_prev:hover,
.default .rich_message .rich_slideshow_next:hover {
    opacity: 1;
}
.default .rich_message .rich_slideshow_dots {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 10px;
    display: flex;
    justify-content: center;
    gap: 6px;
    z-index: 3;
}
.default .rich_message .rich_slideshow_dot {
    width: 8px;
    height: 8px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.5);
    cursor: pointer;
}
.default .rich_message .rich_slideshow_dot.active {
    background: #ffffff;
}
.default .rich_message .rich_embed_post[data-rich-has-items="true"][data-rich-has-meta="false"] > .rich_embed_post_author + .rich_embed_post_blocks {
    margin-top: 3px;
}
.default .rich_message .rich_embed_post[data-rich-has-items="true"][data-rich-has-meta="true"] > .rich_embed_post_meta + .rich_embed_post_blocks {
    margin-top: 8px;
}
.default .rich_message .rich_embed_post[data-rich-has-items="false"][data-rich-has-meta="false"] > .rich_embed_post_blocks + .rich_media_caption,
.default .rich_message .rich_embed_post[data-rich-has-items="true"] > .rich_embed_post_blocks + .rich_media_caption {
    margin-top: 0;
}
.default .rich_message .rich_embed_post[data-rich-has-items="false"][data-rich-has-meta="true"] > .rich_embed_post_blocks:not(:last-child),
.default .rich_message .rich_embed_post[data-rich-has-items="true"] > .rich_embed_post_blocks:not(:last-child) {
    margin-bottom: 5px;
}
.default .rich_message .rich_embed_post[data-rich-has-items="true"][data-rich-items-end-media="true"] > .rich_embed_post_blocks:not(:last-child) {
    margin-bottom: 0;
}
.default .rich_message .rich_related_title {
    width: 100%;
    margin: 0 0 8px;
    padding: 0;
    color: #000000;
    font-size: 14px;
    line-height: 18px;
    font-weight: 700;
    text-align: start;
    unicode-bidi: plaintext;
    white-space: normal;
    overflow-wrap: anywhere;
    word-break: break-word;
}
.default .rich_message .rich_article_description {
    width: 100%;
    margin: 0;
    padding: 0;
    color: #000000;
    font-size: 13px;
    line-height: 18px;
    text-align: start;
    unicode-bidi: plaintext;
    white-space: normal;
    overflow-wrap: anywhere;
    word-break: break-word;
}
.default .rich_message .rich_related_article > .rich_source_meta {
    font-size: 13px;
    line-height: 18px;
}
.default .rich_message .rich_article_description + .rich_source_meta {
    margin-top: 4px;
}

.spoiler {
    background: #e8e8e8;
}
.spoiler.hidden {
    background: #a9a9a9;
    cursor: pointer;
    border-radius: 3px;
}
.spoiler.hidden span {
    opacity: 0;
    user-select: none;
}

.bot_buttons_table {
    border-spacing: 0px 2px;
    width: 100%;
}
.bot_button {
    border-radius: 8px;
    text-align: center;
    vertical-align: middle;
    background-color: #168acd40;
}
.bot_button_row {
    display: table;
    table-layout: fixed;
    padding: 0px;
    width:100%;
}
.bot_button_row div {
    display: table-cell;
}
.bot_button_column_separator {
    width: 2px
}

.reactions {
    margin: 5px 0;
}

.reactions .reaction {
    display: inline-flex;
    height: 20px;
    border-radius: 15px;
    background-color: #e8f5fc;
    color: #168acd;
    font-weight: bold;
    margin-bottom: 5px;
}

.reactions .reaction.active {
    background-color: #40a6e2;
    color: #fff;
}

.reactions .reaction.paid {
    background-color: #fdf6e1;
    color: #c58523;
}

.reactions .reaction.active.paid {
    background-color: #ecae0a;
    color: #fdf6e1;
}

.reactions .reaction .emoji {
    line-height: 20px;
    margin: 0 5px;
    font-size: 15px;
}

.reactions .reaction .userpic:not(:first-child) {
    margin-left: -8px;
}

.reactions .reaction .userpic {
    display: inline-block;
}

.reactions .reaction .userpic .initials {
    font-size: 8px;
}

.reactions .reaction .count {
    margin-right: 8px;
    line-height: 20px;
}

@media (prefers-color-scheme: dark) {
html, body {
    background-color: #1a2026; /* groupCallBg */
    margin: 0;
    padding: 0;
}
.page_wrap {
    background-color: #1a2026; /* groupCallBg */
    color: #ffffff; /* groupCallMembersFg */
    min-height: 100vh;
}
.page_wrap a {
    color: #4db8ff; /* groupCallActiveFg */
}
.page_header {
    background-color: #1a2026; /* groupCallBg */
    border-bottom: 1px solid #2c333d; /* groupCallMembersBg */
}
.bold {
    color: #ffffff; /* groupCallMembersFg */
}
.details {
    color: #91979e; /* groupCallMemberNotJoinedStatus */
}
.page_body {
    background-color: #1a2026; /* groupCallBg */
}
code {
    color: #ff8aac; /* historyPeer6UserpicBg */
    background-color: #2c333d; /* groupCallMembersBg */
}
pre {
    color: #ffffff; /* groupCallMembersFg */
    background-color: #2c333d; /* groupCallMembersBg */
    border: 1px solid #323a45; /* groupCallMembersBgOver */
}
.with_divider {
    border-top: 1px solid #2c333d; /* groupCallMembersBg */
}
a.block_link:hover {
    background-color: #323a45; /* groupCallMembersBgOver */
}
.list_page .entry {
    color: #ffffff; /* groupCallMembersFg */
}
.message {
    color: #ffffff; /* groupCallMembersFg */
}
div.selected {
    background-color: #323a45; /* groupCallMembersBgOver */
}
.default .from_name {
    color: #4db8ff; /* groupCallActiveFg */
}
.default .media .description {
    color: #ffffff; /* groupCallMembersFg */
}
.default .rich_message .rich_heading {
    color: #ffffff;
}
.default .rich_message .rich_author_date,
.default .rich_message .rich_footer,
.default .rich_message .rich_thinking,
.default .rich_message .rich_quote_caption,
.default .rich_message .rich_table caption,
.default .rich_message .rich_fallback,
.default .rich_message .rich_deferred,
.default .rich_message .rich_inline_fallback {
    color: #91979e;
}
.default .rich_message .rich_quote {
    border-inline-start-color: #4db8ff;
}
.default .rich_message .rich_divider {
    border-top-color: #2c333d;
}
.default .rich_message .rich_details {
    border-bottom-color: #2c333d;
}
.default .rich_message .rich_fallback,
.default .rich_message .rich_deferred {
    background-color: #323a45;
    border-color: #2c333d;
}
.default .rich_message mark,
.default .rich_message .rich_inline_fallback,
.default .rich_message .rich_table th,
.default .rich_message .rich_table.striped tbody > tr:nth-child(even) > td {
    background-color: #323a45;
}
.default .rich_message .rich_code {
    color: #ffffff;
    background-color: #2c333d;
    border-color: #323a45;
}
.default .rich_message .rich_inline_code,
.default .rich_message .rich_math_inline {
    color: #ff8aac;
    background-color: #2c333d;
}
.default .rich_message .rich_math_display {
    color: #ffffff;
    background-color: #2c333d;
}
.default .rich_message .rich_table.bordered,
.default .rich_message .rich_table.bordered th,
.default .rich_message .rich_table.bordered td {
    border-color: #2c333d;
}
.default .rich_message .rich_media_caption,
.default .rich_message .rich_caption_text,
.default .rich_message .rich_caption_credit,
.default .rich_message .rich_source_meta,
.default .rich_message .rich_embed_post_meta,
.default .rich_message .rich_source_text {
    color: #91979e;
}
.default .rich_message .rich_related_title,
.default .rich_message .rich_article_description {
    color: #ffffff;
}
.default .rich_message .rich_source_link {
    color: #4db8ff;
}
.default .rich_message .rich_button_style_default {
    color: #4db8ff;
    background-color: #4db8ff1f;
}
.default .rich_message .rich_button_style_primary {
    color: #1a2026;
    background-color: #4db8ff;
}
.default .rich_message .rich_button_style_success {
    color: #64bf47;
    background-color: #64bf471f;
}
.default .rich_message .rich_button_style_danger {
    color: #ff5555;
    background-color: #ff55551f;
}
.default .rich_message .rich_button_inline.rich_button_style_link {
    color: #4db8ff;
    background: none;
}
msgInBg,
.historyComposeAreaBg {
    background-color: #2c333d; /* groupCallMembersBg */
}
msgOutBg {
    background-color: #323a45; /* groupCallMembersBgOver */
}
msgInBgSelected {
    background-color: #39424f; /* groupCallMembersBgRipple */
}
msgOutBgSelected {
    background-color: #39424f; /* groupCallMembersBgRipple */
}
.spoiler {
    background: #323a45; /* groupCallMembersBgOver */
}
.spoiler.hidden {
    background: #61c0ff; /* groupCallMemberInactiveStatus */
}
.bot_button {
    background-color: #4db8ff40; /* groupCallActiveFg with opacity */
}
.reactions .reaction {
    background-color: #2c333d; /* groupCallMembersBg */
    color: #4db8ff; /* groupCallActiveFg */
}
.reactions .reaction.active {
    background-color: #4db8ff; /* groupCallActiveFg */
    color: #1a2026; /* groupCallBg */
}
.reactions .reaction.paid {
    background-color: #323a45; /* groupCallMembersBgOver */
    color: #febb5b; /* historyPeer8UserpicBg */
}
.reactions .reaction.active.paid {
    background-color: #febb5b; /* historyPeer8UserpicBg */
    color: #1a2026; /* groupCallBg */
}
}
"########;
