//! Desktop rich text, block and button model. GPL-3.0; Telegram Desktop 7.2.10.
use super::model::{Document, Photo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InlineButtonPeerType {
    #[default]
    SameBotPM,
    PM,
    Chat,
    Megagroup,
    Broadcast,
    BotPM,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InlineButtonActionType {
    #[default]
    Url,
    Auth,
    WebView,
    Callback,
    CallbackWithPassword,
    Game,
    Buy,
    SwitchInline,
    SwitchInlineSame,
    UserProfile,
    CopyText,
    Disabled,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InlineButtonAction {
    pub url: String,
    pub forward_text: Option<String>,
    pub callback_data: Vec<u8>,
    pub query: String,
    pub peer_types: Option<Vec<InlineButtonPeerType>>,
    pub copy_text: String,
    pub user_id: i64,
    pub button_id: i64,
    pub r#type: InlineButtonActionType,
    pub requires_password: bool,
    pub same_peer: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichButtonStyle {
    #[default]
    Default,
    Primary,
    Success,
    Danger,
    Link,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichButtonAlignment {
    #[default]
    Stretch,
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichButtonPayload {
    pub action: InlineButtonAction,
    pub style: Option<RichButtonStyle>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichTextType {
    #[default]
    Empty,
    Plain,
    Concat,
    Bold,
    Italic,
    Underline,
    Strike,
    Fixed,
    Url,
    Email,
    Phone,
    Subscript,
    Superscript,
    Marked,
    Anchor,
    Math,
    CustomEmoji,
    Spoiler,
    Mention,
    Hashtag,
    BotCommand,
    Cashtag,
    AutoUrl,
    AutoEmail,
    AutoPhone,
    BankCard,
    TonAddress,
    MentionName,
    FormattedDate,
    InlineImage,
    Diff,
    Button,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichText {
    pub text: String,
    pub data: String,
    pub custom_emoji_data: String,
    pub children: Vec<RichText>,
    pub old_children: Vec<RichText>,
    pub button: Option<RichButtonPayload>,
    pub id: i64,
    pub date: i64,
    pub width: i64,
    pub height: i64,
    pub r#type: RichTextType,
    pub relative: bool,
    pub short_time: bool,
    pub long_time: bool,
    pub short_date: bool,
    pub long_date: bool,
    pub day_of_week: bool,
    pub unsupported: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichCaption {
    pub text: RichText,
    pub credit: RichText,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichTaskState {
    #[default]
    None_,
    Unchecked,
    Checked,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichListKind {
    #[default]
    Bullet,
    Ordered,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichListItemContent {
    #[default]
    Text,
    Blocks,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichQuoteContent {
    #[default]
    Text,
    Blocks,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichOrderedList {
    pub r#type: Option<String>,
    pub start: Option<i64>,
    pub reversed: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichListItem {
    pub text: Option<RichText>,
    pub blocks: Vec<RichBlock>,
    pub num: Option<String>,
    pub r#type: Option<String>,
    pub value: Option<i64>,
    pub task_state: RichTaskState,
    pub content: RichListItemContent,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichTableAlignment {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichTableVerticalAlignment {
    #[default]
    Top,
    Middle,
    Bottom,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichTableCell {
    pub text: Option<RichText>,
    pub colspan: Option<i64>,
    pub rowspan: Option<i64>,
    pub alignment: RichTableAlignment,
    pub vertical_alignment: RichTableVerticalAlignment,
    pub header: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichTableRow {
    pub cells: Vec<RichTableCell>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichRelatedArticle {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub photo_id: Option<i64>,
    pub published_date: Option<i64>,
    pub webpage_id: i64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichChannelSource {
    #[default]
    ChatEmpty,
    Chat,
    ChatForbidden,
    Channel,
    ChannelForbidden,
    Community,
    CommunityForbidden,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichChannel {
    pub title: Option<String>,
    pub username: Option<String>,
    pub access_hash: Option<i64>,
    pub id: i64,
    pub source: RichChannelSource,
    pub broadcast: bool,
    pub megagroup: bool,
    pub monoforum: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichMapPointSource {
    #[default]
    GeoPointEmpty,
    GeoPoint,
    InputGeoPointEmpty,
    InputGeoPoint,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichMapPoint {
    pub access_hash: Option<i64>,
    pub accuracy_radius: Option<i64>,
    pub latitude: f64,
    pub longitude: f64,
    pub source: RichMapPointSource,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichBlockKind {
    Unsupported,
    Heading,
    Paragraph,
    Footer,
    Thinking,
    AuthorDate,
    Code,
    Divider,
    Anchor,
    List,
    Quote,
    Photo,
    Video,
    Cover,
    Embed,
    EmbedPost,
    Collage,
    Slideshow,
    Channel,
    Audio,
    File,
    Math,
    Table,
    Details,
    RelatedArticles,
    Map,
    InputMap,
    ButtonRow,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichBlock {
    pub text: RichText,
    pub quote_caption: RichText,
    pub caption: RichCaption,
    pub ordered_list: RichOrderedList,
    pub channel: RichChannel,
    pub map_point: RichMapPoint,
    pub language: String,
    pub formula: String,
    pub name: String,
    pub url: String,
    pub author: String,
    pub optional_url: Option<String>,
    pub html: Option<String>,
    pub blocks: Vec<RichBlock>,
    pub list_items: Vec<RichListItem>,
    pub table_rows: Vec<RichTableRow>,
    pub related_articles: Vec<RichRelatedArticle>,
    pub buttons: Vec<RichText>,
    pub optional_webpage_id: Option<i64>,
    pub poster_photo_id: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub photo_id: i64,
    pub document_id: i64,
    pub webpage_id: i64,
    pub author_photo_id: i64,
    pub date: i64,
    pub heading_level: i64,
    pub zoom: i64,
    pub map_width: i64,
    pub map_height: i64,
    pub kind: RichBlockKind,
    pub list_kind: RichListKind,
    pub quote_content: RichQuoteContent,
    pub button_alignment: RichButtonAlignment,
    pub unsupported: bool,
    pub full_width: bool,
    pub allow_scrolling: bool,
    pub autoplay: bool,
    pub r#loop: bool,
    pub spoiler: bool,
    pub open: bool,
    pub bordered: bool,
    pub striped: bool,
    pub compact: bool,
    pub pullquote: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichMessage {
    pub blocks: Vec<RichBlock>,
    pub photos: BTreeMap<i64, Photo>,
    pub documents: BTreeMap<i64, Document>,
    pub rtl: bool,
    pub part: bool,
}
