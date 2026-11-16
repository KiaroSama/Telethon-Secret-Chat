//! Telegram Desktop export css/style.css static data, GPL-3.0.
pub const DATA: &str = r########".section.other {
    background-image: url(../images/section_other.png)
}
.page_header a.content {
    background-image: url(../images/back.png);
}
.media_call .fill {
    background-image: url(../images/media_call.png)
}
.media_contact .fill {
    background-image: url(../images/media_contact.png)
}
.media_file .fill {
    background-image: url(../images/media_file.png)
}
.media_game .fill {
    background-image: url(../images/media_game.png)
}
.media_live_location .fill,
.media_location .fill,
.media_venue .fill {
    background-image: url(../images/media_location.png)
}
.media_audio_file .fill {
    background-image: url(../images/media_music.png)
}
.media_invoice .fill {
    background-image: url(../images/media_shop.png)
}
.media_voice_message .fill {
    background-image: url(../images/media_voice.png)
}
.media_photo .fill {
    background-image: url(../images/media_photo.png)
}
.media_video .fill {
    background-image: url(../images/media_video.png)
}
.audio_icon {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background-color: #4f9cd9;
    background-image: url(../images/media_music.png);
    background-repeat: no-repeat;
    background-position: 12px 12px;
    background-size: 24px 24px;
}

@media only screen and (min--moz-device-pixel-ratio: 2), only screen and (-o-min-device-pixel-ratio: 2/1), only screen and (-webkit-min-device-pixel-ratio: 2), only screen and (min-device-pixel-ratio: 2) {
.section.calls {
    background-image: url(../images/section_calls@2x.png);
}
.section.chats {
    background-image: url(../images/section_chats@2x.png);
}
.section.contacts {
    background-image: url(../images/section_contacts@2x.png);
}
.section.frequent {
    background-image: url(../images/section_frequent@2x.png);
}
.section.photos {
    background-image: url(../images/section_photos@2x.png);
}
.section.sessions {
    background-image: url(../images/section_sessions@2x.png);
}
.section.stories {
    background-image: url(../images/section_stories@2x.png);
}
.section.music {
    background-image: url(../images/section_music@2x.png);
}
.section.web {
    background-image: url(../images/section_web@2x.png);
}
.section.other {
    background-image: url(../images/section_other@2x.png);
}
.page_header a.content {
    background-image: url(../images/back@2x.png);
}
.media_call .fill {
    background-image: url(../images/media_call@2x.png)
}
.media_contact .fill {
    background-image: url(../images/media_contact@2x.png)
}
.media_file .fill {
    background-image: url(../images/media_file@2x.png)
}
.media_game .fill {
    background-image: url(../images/media_game@2x.png)
}
.media_live_location .fill,
.media_location .fill,
.media_venue .fill {
    background-image: url(../images/media_location@2x.png)
}
.media_audio_file .fill {
    background-image: url(../images/media_music@2x.png)
}
.media_invoice .fill {
    background-image: url(../images/media_shop@2x.png)
}
.media_voice_message .fill {
    background-image: url(../images/media_voice@2x.png)
}
.media_photo .fill {
    background-image: url(../images/media_photo@2x.png)
}
.media_video .fill {
    background-image: url(../images/media_video@2x.png)
}
.audio_icon {
    background-image: url(../images/media_music@2x.png);
}
}

.default .rich_message {
    min-width: 0;
    max-width: 100%;
    overflow-wrap: anywhere;
    word-break: break-word;
    line-height: 150%;
    unicode-bidi: plaintext;
    text-align: start;
}
.default .rich_message,
.default .rich_message * {
    box-sizing: border-box;
}
.default .rich_message .rich_block {
    min-width: 0;
    max-width: 100%;
    font-size: 12px;
    line-height: 18px;
}
.default .rich_message .rich_list_content,
.default .rich_message .rich_quote_body,
.default .rich_message .rich_quote_caption,
.default .rich_message .rich_details_body,
.default .rich_message .rich_deferred_label {
    min-width: 0;
    max-width: 100%;
}
.default .rich_message .rich_button_row {
    display: flex;
    min-width: 0;
    max-width: 100%;
    margin: 0 0 8px;
}
.default .rich_message .rich_button_group {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    min-width: 0;
    max-width: 100%;
}
.default .rich_message .rich_button_align_stretch .rich_button_group {
    align-items: flex-start;
    width: 100%;
}
.default .rich_message .rich_button_align_left {
    justify-content: flex-start;
}
.default .rich_message .rich_button_align_center {
    justify-content: center;
}
.default .rich_message .rich_button_align_right {
    justify-content: flex-end;
}
.default .rich_message .rich_button_align_left .rich_button_group,
.default .rich_message .rich_button_align_center .rich_button_group,
.default .rich_message .rich_button_align_right .rich_button_group {
    width: fit-content;
}
.default .rich_message .rich_button_align_left .rich_button_group {
    justify-content: flex-start;
}
.default .rich_message .rich_button_align_center .rich_button_group {
    justify-content: center;
}
.default .rich_message .rich_button_align_right .rich_button_group {
    justify-content: flex-end;
}
.default .rich_message .rich_button {
    appearance: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    max-width: 100%;
    margin: 0;
    padding: 0.35em 0.75em;
    color: inherit;
    border: 0;
    border-radius: 1em;
    font: inherit;
    font-weight: 600;
    line-height: inherit;
    text-align: center;
    text-decoration: none;
    vertical-align: middle;
    white-space: normal;
    cursor: default;
    overflow-wrap: anywhere;
    word-break: break-word;
}
.default .rich_message .rich_button_block {
    flex: 0 1 auto;
}
.default .rich_message .rich_button_align_stretch .rich_button_block {
    flex: 1 1 6em;
}
.default .rich_message .rich_button_inline {
    padding: 0.1em 0.5em;
}
.default .rich_message a.rich_button_action_url,
.default .rich_message button.rich_button_action_copy_text {
    cursor: pointer;
}
.default .rich_message .rich_button_style_default {
    color: #168acd;
    background-color: #168acd1f;
}
.default .rich_message .rich_button_style_primary {
    color: #ffffff;
    background-color: #168acd;
}
.default .rich_message .rich_button_style_success {
    color: #64bf47;
    background-color: #64bf471f;
}
.default .rich_message .rich_button_style_danger {
    color: #ff5555;
    background-color: #ff55551f;
}
.default .rich_message .rich_button_action_disabled {
    opacity: 0.33;
    cursor: default;
}
.default .rich_message .rich_button_inline.rich_button_style_link {
    display: inline;
    padding: 0;
    color: #168acd;
    background: none;
    border: 0;
    border-radius: 0;
    box-shadow: none;
    font: inherit;
    line-height: inherit;
    text-decoration: none;
}
.default .rich_message > .rich_block:first-child {
    margin-top: 0;
}
.default .rich_message > .rich_block:last-child {
    margin-bottom: 0;
}
.default .rich_message .rich_list_content > .rich_block:not(.rich_list):first-child,
.default .rich_message .rich_quote_body > .rich_block:not(.rich_list):first-child,
.default .rich_message .rich_details_body > .rich_block:not(.rich_list):first-child,
.default .rich_message .rich_deferred_label + .rich_block:not(.rich_list) {
    margin-top: 0;
}
.default .rich_message .rich_list_content > .rich_block:not(.rich_list):last-child,
.default .rich_message .rich_quote_body > .rich_block:not(.rich_list):last-child,
.default .rich_message .rich_details_body > .rich_block:not(.rich_list):last-child,
.default .rich_message .rich_deferred > .rich_block:not(.rich_list):last-child {
    margin-bottom: 0;
}
.default .rich_message .rich_heading {
    margin: 12px 0 6px;
    font-weight: 700;
    text-align: start;
    overflow-wrap: anywhere;
}
.default .rich_message h1.rich_heading {
    font-size: 20px;
    line-height: 26px;
}
.default .rich_message h2.rich_heading {
    font-size: 18px;
    line-height: 24px;
}
.default .rich_message h3.rich_heading {
    font-size: 16px;
    line-height: 22px;
}
.default .rich_message h4.rich_heading {
    font-size: 15px;
    line-height: 21px;
}
.default .rich_message h5.rich_heading {
    font-size: 14px;
    line-height: 20px;
}
.default .rich_message h6.rich_heading {
    font-size: 13px;
    line-height: 19px;
}
.default .rich_message .rich_paragraph {
    margin: 0 0 8px;
    font-size: 12px;
    line-height: 18px;
}
.default .rich_message .rich_author_date {
    margin: 0 0 8px;
    color: #70777b;
    font-size: 12px;
    line-height: 18px;
    font-style: normal;
}
.default .rich_message .rich_author_date time {
    direction: ltr;
    unicode-bidi: isolate;
    white-space: nowrap;
}
.default .rich_message .rich_formatted_date {
    direction: ltr;
    unicode-bidi: isolate;
}
.default .rich_message .rich_footer,
.default .rich_message .rich_thinking {
    margin: 0 0 8px;
    color: #70777b;
    font-size: 11px;
    line-height: 17px;
}
.default .rich_message .rich_quote_caption,
.default .rich_message .rich_table caption {
    color: #70777b;
    font-size: 11px;
    line-height: 17px;
    font-style: normal;
}
.default .rich_message .rich_divider {
    width: 50%;
    height: 0;
    margin: 12px auto;
    padding: 0;
    border: 0;
    border-top: 1px solid #e3e6e8;
}
.default .rich_message .rich_anchor {
    display: block;
    width: 0;
    height: 0;
    margin: 0;
    padding: 0;
    border: 0;
}
.default .rich_message mark {
    padding: 0 2px;
    color: inherit;
    background-color: #f5f7f8;
    border-radius: 2px;
}
.default .rich_message sub,
.default .rich_message sup {
    font-size: 75%;
    line-height: 1;
}
.default .rich_message .rich_inline_code {
    display: inline;
    max-width: 100%;
    padding: 2px 4px;
    font-size: 90%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    word-break: break-word;
    border-radius: 4px;
}
.default .rich_message .rich_code {
    display: block;
    min-width: 0;
    max-width: 100%;
    max-height: none;
    margin: 8px 0;
    padding: 6px;
    border: 1px solid #eeeeee;
    border-radius: 4px;
    font-size: inherit;
    line-height: 1.42857143;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    word-break: break-word;
    overflow-x: auto;
}
.default .rich_message .rich_code_content {
    display: inline;
    padding: 0;
    color: inherit;
    background-color: transparent;
    border: 0;
    border-radius: 0;
    font-size: inherit;
    line-height: inherit;
    white-space: inherit;
    overflow-wrap: inherit;
    word-break: inherit;
}
.default .rich_message .rich_list {
    margin: 0 0 8px;
    padding: 0;
    padding-inline-start: 20px;
}
.default .rich_message .rich_list .rich_list {
    margin: 4px 0;
    padding: 0;
    padding-inline-start: 20px;
}
.default .rich_message .rich_list_item + .rich_list_item {
    margin-top: 4px;
}
.default .rich_message .rich_task_item,
.default .rich_message .rich_order_item {
    display: flex;
    flex-direction: row;
    align-items: flex-start;
    gap: 6px;
    list-style: none;
}
.default .rich_message .rich_task_marker {
    display: inline-flex;
    flex: 0 0 14px;
    align-items: center;
    justify-content: center;
    width: 14px;
    min-width: 14px;
    max-width: 14px;
    height: 14px;
    margin-block-start: 2px;
    line-height: 14px;
}
.default .rich_message .rich_order_marker {
    display: inline-flex;
    flex: 0 1 auto;
    min-inline-size: 14px;
    max-inline-size: 84px;
    overflow-wrap: anywhere;
    word-break: break-word;
}
.default .rich_message .rich_list_content {
    display: block;
    flex: 1 1 auto;
    min-width: 0;
    max-width: 100%;
"########;
