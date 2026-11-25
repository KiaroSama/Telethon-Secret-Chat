//! Native constructor names for official Grammers 02367f97 (MIT OR Apache-2.0).
//! Enum unit variants and Rust acronym normalization are distinct from TL constructor spelling.
#[path = "tl_names_0.rs"]
mod first;
#[path = "tl_names_1.rs"]
mod second;
pub(super) fn unit_constructor(name: &str, variant: &str) -> Option<&'static str> {
    [first::UNITS, second::UNITS].into_iter().find_map(|units| {
        units
            .binary_search_by_key(&(name, variant), |row| row.0)
            .ok()
            .map(|index| units[index].1)
    })
}
pub(super) fn canonical_name(name: &str) -> &str {
    CANONICAL
        .binary_search_by_key(&name, |row| row.0)
        .map_or(name, |index| CANONICAL[index].1)
}

const CANONICAL: &[(&str, &str)] = &[
    ("AccountDaysTtl", "AccountDaysTTL"),
    ("AttachMenuPeerTypeBotPm", "AttachMenuPeerTypeBotPM"),
    ("AttachMenuPeerTypePm", "AttachMenuPeerTypePM"),
    ("AttachMenuPeerTypeSameBotPm", "AttachMenuPeerTypeSameBotPM"),
    (
        "ChannelAdminLogEventActionChangeHistoryTtl",
        "ChannelAdminLogEventActionChangeHistoryTTL",
    ),
    ("ComposedMessageWithAi", "ComposedMessageWithAI"),
    ("ComposedRichMessageWithAi", "ComposedRichMessageWithAI"),
    ("DataJson", "DataJSON"),
    ("DefaultHistoryTtl", "DefaultHistoryTTL"),
    ("EmojiUrl", "EmojiURL"),
    ("InlineBotSwitchPm", "InlineBotSwitchPM"),
    ("InlineQueryPeerTypeBotPm", "InlineQueryPeerTypeBotPM"),
    ("InlineQueryPeerTypePm", "InlineQueryPeerTypePM"),
    (
        "InlineQueryPeerTypeSameBotPm",
        "InlineQueryPeerTypeSameBotPM",
    ),
    ("InputAiComposeToneId", "InputAiComposeToneID"),
    ("InputBotAppId", "InputBotAppID"),
    ("InputBotInlineMessageId", "InputBotInlineMessageID"),
    ("InputBotInlineMessageId64", "InputBotInlineMessageID64"),
    ("InputCheckPasswordSrp", "InputCheckPasswordSRP"),
    ("InputGameId", "InputGameID"),
    ("InputMessageId", "InputMessageID"),
    ("InputRichMessageHtml", "InputRichMessageHTML"),
    ("InputStickerSetId", "InputStickerSetID"),
    ("MessageActionSetMessagesTtl", "MessageActionSetMessagesTTL"),
    (
        "PasswordKdfAlgoSha256Sha256Pbkdf2Hmacsha512iter100000Sha256ModPow",
        "PasswordKdfAlgoSHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow",
    ),
    ("ResPq", "ResPQ"),
    (
        "SecurePasswordKdfAlgoPbkdf2Hmacsha512iter100000",
        "SecurePasswordKdfAlgoPBKDF2HMACSHA512iter100000",
    ),
    ("SecurePasswordKdfAlgoSha512", "SecurePasswordKdfAlgoSHA512"),
    ("StarsTransactionPeerApi", "StarsTransactionPeerAPI"),
    ("StatsUrl", "StatsURL"),
    ("TopPeerCategoryBotsPm", "TopPeerCategoryBotsPM"),
    ("UpdateBotWebhookJson", "UpdateBotWebhookJSON"),
    ("UpdateBotWebhookJsonquery", "UpdateBotWebhookJSONQuery"),
    ("UpdateMessageId", "UpdateMessageID"),
    ("UpdatePeerHistoryTtl", "UpdatePeerHistoryTTL"),
    ("UpdateStoryId", "UpdateStoryID"),
];
