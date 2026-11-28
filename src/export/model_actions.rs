//! Desktop service action variants. Ported from Telegram Desktop 7.2.10 (GPL-3.0).
use super::{model::*, model_media::TodoListItem};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhoneCallState {
    #[default]
    Unknown,
    Missed,
    Disconnect,
    Hangup,
    Busy,
    MigrateConferenceCall,
    Invitation,
    Active,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecureValueType {
    PersonalDetails,
    Passport,
    DriverLicense,
    IdentityCard,
    InternalPassport,
    Address,
    UtilityBill,
    BankStatement,
    RentalAgreement,
    PassportRegistration,
    TemporaryRegistration,
    Phone,
    Email,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatCreate {
    pub title: String,
    pub user_ids: Vec<i64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatEditTitle {
    pub title: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatEditPhoto {
    pub photo: Photo,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatDeletePhoto {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatAddUser {
    pub user_ids: Vec<i64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatDeleteUser {
    pub user_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatJoinedByLink {
    pub inviter_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChannelCreate {
    pub title: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatMigrateTo {
    pub channel_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChannelMigrateFrom {
    pub title: String,
    pub chat_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPinMessage {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionHistoryClear {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGameScore {
    pub game_id: i64,
    pub score: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPaymentSent {
    pub currency: String,
    pub amount: i64,
    pub recurring_init: bool,
    pub recurring_used: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPhoneCall {
    pub conference_id: i64,
    pub state: PhoneCallState,
    pub duration: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionScreenshotTaken {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionCustomAction {
    pub message: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionBotAllowed {
    pub app_id: i64,
    pub app: String,
    pub domain: String,
    pub attach_menu: bool,
    pub from_request: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSecureValuesSent {
    pub types: Vec<SecureValueType>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionContactSignUp {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPhoneNumberRequest {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGeoProximityReached {
    pub from_id: i64,
    pub to_id: i64,
    pub distance: i64,
    pub from_self: bool,
    pub to_self: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGroupCall {
    pub duration: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionInviteToGroupCall {
    pub user_ids: Vec<i64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSetMessagesTTL {
    pub period: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGroupCallScheduled {
    pub date: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSetChatTheme {
    pub emoji: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatJoinedByRequest {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChatJoinedViaCommunity {
    pub community_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionWebViewDataSent {
    pub text: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGiftPremium {
    pub cost: String,
    pub days: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionTopicCreate {
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionTopicEdit {
    pub title: String,
    pub icon_emoji_id: Option<i64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSuggestProfilePhoto {
    pub photo: Photo,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSetChatWallPaper {
    pub same: bool,
    pub both: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGiftCode {
    pub code: String,
    pub boost_peer_id: i64,
    pub days: i64,
    pub via_giveaway: bool,
    pub unclaimed: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionRequestedPeer {
    pub peers: Vec<i64>,
    pub button_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGiveawayLaunch {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGiveawayResults {
    pub winners: i64,
    pub unclaimed: i64,
    pub credits: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionBoostApply {
    pub boosts: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPaymentRefunded {
    pub peer_id: i64,
    pub currency: String,
    pub amount: i64,
    pub transaction_id: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGiftCredits {
    pub cost: String,
    pub amount: CreditsAmount,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPrizeStars {
    pub peer_id: i64,
    pub amount: i64,
    pub transaction_id: String,
    pub giveaway_msg_id: i64,
    pub is_unclaimed: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionStarGift {
    pub gift_id: i64,
    pub stars: i64,
    pub text: Vec<TextPart>,
    pub anonymous: bool,
    pub limited: bool,
    pub offer_price: CreditsAmount,
    pub offer_expire_at: i64,
    pub offer: bool,
    pub offer_accepted: bool,
    pub offer_declined: bool,
    pub offer_expired: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPaidMessagesRefunded {
    pub messages: i64,
    pub stars: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPaidMessagesPrice {
    pub stars: i64,
    pub broadcast_allowed: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionTodoCompletions {
    pub completed: Vec<i64>,
    pub incompleted: Vec<i64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionTodoAppendTasks {
    pub items: Vec<TodoListItem>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPollAppendAnswer {
    pub option: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionPollDeleteAnswer {
    pub option: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSuggestedPostApproval {
    pub reject_comment: String,
    pub schedule_date: i64,
    pub price: CreditsAmount,
    pub rejected: bool,
    pub balance_too_low: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSuggestedPostSuccess {
    pub price: CreditsAmount,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSuggestedPostRefund {
    pub payer_initiated: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSuggestBirthday {
    pub birthday: Birthday,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionNoForwardsToggle {
    pub new_value: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionNoForwardsRequest {
    pub expired: bool,
    pub new_value: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionNewCreatorPending {
    pub new_creator_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionChangeCreator {
    pub new_creator_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionManagedBotCreated {
    pub bot_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionGramTransfer {
    pub amount: i64,
    pub peer_address: String,
    pub transaction_id: String,
    pub comment: String,
    pub comment_encrypted: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub onramp_provider_name: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionBotInvite {
    pub inviter_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionWalletTonConnectRequest {
    pub session_id: u64,
    pub expires: i64,
    pub topic: String,
    pub trace_id: String,
    pub accepted: bool,
    pub declined: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionNoPaidMessagesTokenUsed {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    ChatCreate(ActionChatCreate),
    ChatEditTitle(ActionChatEditTitle),
    ChatEditPhoto(ActionChatEditPhoto),
    ChatDeletePhoto(ActionChatDeletePhoto),
    ChatAddUser(ActionChatAddUser),
    ChatDeleteUser(ActionChatDeleteUser),
    ChatJoinedByLink(ActionChatJoinedByLink),
    ChannelCreate(ActionChannelCreate),
    ChatMigrateTo(ActionChatMigrateTo),
    ChannelMigrateFrom(ActionChannelMigrateFrom),
    PinMessage(ActionPinMessage),
    HistoryClear(ActionHistoryClear),
    GameScore(ActionGameScore),
    PaymentSent(ActionPaymentSent),
    PhoneCall(ActionPhoneCall),
    ScreenshotTaken(ActionScreenshotTaken),
    CustomAction(ActionCustomAction),
    BotAllowed(ActionBotAllowed),
    SecureValuesSent(ActionSecureValuesSent),
    ContactSignUp(ActionContactSignUp),
    PhoneNumberRequest(ActionPhoneNumberRequest),
    GeoProximityReached(ActionGeoProximityReached),
    GroupCall(ActionGroupCall),
    InviteToGroupCall(ActionInviteToGroupCall),
    SetMessagesTTL(ActionSetMessagesTTL),
    GroupCallScheduled(ActionGroupCallScheduled),
    SetChatTheme(ActionSetChatTheme),
    ChatJoinedByRequest(ActionChatJoinedByRequest),
    ChatJoinedViaCommunity(ActionChatJoinedViaCommunity),
    WebViewDataSent(ActionWebViewDataSent),
    GiftPremium(ActionGiftPremium),
    TopicCreate(ActionTopicCreate),
    TopicEdit(ActionTopicEdit),
    SuggestProfilePhoto(ActionSuggestProfilePhoto),
    SetChatWallPaper(ActionSetChatWallPaper),
    GiftCode(ActionGiftCode),
    RequestedPeer(ActionRequestedPeer),
    GiveawayLaunch(ActionGiveawayLaunch),
    GiveawayResults(ActionGiveawayResults),
    BoostApply(ActionBoostApply),
    PaymentRefunded(ActionPaymentRefunded),
    GiftCredits(ActionGiftCredits),
    PrizeStars(ActionPrizeStars),
    StarGift(ActionStarGift),
    PaidMessagesRefunded(ActionPaidMessagesRefunded),
    PaidMessagesPrice(ActionPaidMessagesPrice),
    TodoCompletions(ActionTodoCompletions),
    TodoAppendTasks(ActionTodoAppendTasks),
    PollAppendAnswer(ActionPollAppendAnswer),
    PollDeleteAnswer(ActionPollDeleteAnswer),
    SuggestedPostApproval(ActionSuggestedPostApproval),
    SuggestedPostSuccess(ActionSuggestedPostSuccess),
    SuggestedPostRefund(ActionSuggestedPostRefund),
    SuggestBirthday(ActionSuggestBirthday),
    NoForwardsToggle(ActionNoForwardsToggle),
    NoForwardsRequest(ActionNoForwardsRequest),
    NewCreatorPending(ActionNewCreatorPending),
    ChangeCreator(ActionChangeCreator),
    ManagedBotCreated(ActionManagedBotCreated),
    GramTransfer(ActionGramTransfer),
    BotInvite(ActionBotInvite),
    WalletTonConnectRequest(ActionWalletTonConnectRequest),
    NoPaidMessagesTokenUsed(ActionNoPaidMessagesTokenUsed),
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceAction {
    pub content: Option<Action>,
}
impl Default for ActionTopicEdit {
    fn default() -> Self {
        Self {
            title: String::new(),
            icon_emoji_id: Some(0),
        }
    }
}
