//! Native Grammers 02367f97 constructor-name data (MIT OR Apache-2.0).
pub(super) const UNITS: &[((&str, &str), &str)] = &[
    (("Albums", "NotModified"), "AlbumsNotModified"),
    (("AllStickers", "NotModified"), "AllStickersNotModified"),
    (("AppConfig", "NotModified"), "AppConfigNotModified"),
    (("AppUpdate", "NoAppUpdate"), "NoAppUpdate"),
    (
        ("AttachMenuBots", "NotModified"),
        "AttachMenuBotsNotModified",
    ),
    (("AttachMenuPeerType", "BotPm"), "AttachMenuPeerTypeBotPm"),
    (
        ("AttachMenuPeerType", "Broadcast"),
        "AttachMenuPeerTypeBroadcast",
    ),
    (("AttachMenuPeerType", "Chat"), "AttachMenuPeerTypeChat"),
    (("AttachMenuPeerType", "Pm"), "AttachMenuPeerTypePm"),
    (
        ("AttachMenuPeerType", "SameBotPm"),
        "AttachMenuPeerTypeSameBotPm",
    ),
    (
        ("AvailableEffects", "NotModified"),
        "AvailableEffectsNotModified",
    ),
    (
        ("AvailableReactions", "NotModified"),
        "AvailableReactionsNotModified",
    ),
    (("BaseTheme", "Arctic"), "BaseThemeArctic"),
    (("BaseTheme", "Classic"), "BaseThemeClassic"),
    (("BaseTheme", "Day"), "BaseThemeDay"),
    (("BaseTheme", "Night"), "BaseThemeNight"),
    (("BaseTheme", "Tinted"), "BaseThemeTinted"),
    (("BotApp", "NotModified"), "BotAppNotModified"),
    (
        ("BotCommandScope", "ChatAdmins"),
        "BotCommandScopeChatAdmins",
    ),
    (("BotCommandScope", "Chats"), "BotCommandScopeChats"),
    (("BotCommandScope", "Default"), "BotCommandScopeDefault"),
    (("BotCommandScope", "Users"), "BotCommandScopeUsers"),
    (("BotMenuButton", "Commands"), "BotMenuButtonCommands"),
    (("BotMenuButton", "Default"), "BotMenuButtonDefault"),
    (
        ("BusinessAwayMessageSchedule", "Always"),
        "BusinessAwayMessageScheduleAlways",
    ),
    (
        ("BusinessAwayMessageSchedule", "OutsideWorkHours"),
        "BusinessAwayMessageScheduleOutsideWorkHours",
    ),
    (("ButtonType", "Default"), "ButtonTypeDefault"),
    (
        ("ButtonType", "RequestGeoLocation"),
        "ButtonTypeRequestGeoLocation",
    ),
    (("ButtonType", "RequestPhone"), "ButtonTypeRequestPhone"),
    (
        ("ChannelAdminLogEventAction", "ParticipantJoin"),
        "ChannelAdminLogEventActionParticipantJoin",
    ),
    (
        ("ChannelAdminLogEventAction", "ParticipantLeave"),
        "ChannelAdminLogEventActionParticipantLeave",
    ),
    (("ChannelLocation", "Empty"), "ChannelLocationEmpty"),
    (
        ("ChannelMessagesFilter", "Empty"),
        "ChannelMessagesFilterEmpty",
    ),
    (
        ("ChannelParticipants", "NotModified"),
        "ChannelParticipantsNotModified",
    ),
    (
        ("ChannelParticipantsFilter", "ChannelParticipantsAdmins"),
        "ChannelParticipantsAdmins",
    ),
    (
        ("ChannelParticipantsFilter", "ChannelParticipantsBots"),
        "ChannelParticipantsBots",
    ),
    (
        ("ChannelParticipantsFilter", "ChannelParticipantsRecent"),
        "ChannelParticipantsRecent",
    ),
    (("ChatPhoto", "Empty"), "ChatPhotoEmpty"),
    (("ChatReactions", "None"), "ChatReactionsNone"),
    (("ChatThemes", "NotModified"), "ChatThemesNotModified"),
    (("CheckCanSendGiftResult", "Ok"), "CheckCanSendGiftResultOk"),
    (("CodeType", "Call"), "CodeTypeCall"),
    (("CodeType", "FlashCall"), "CodeTypeFlashCall"),
    (("CodeType", "FragmentSms"), "CodeTypeFragmentSms"),
    (("CodeType", "MissedCall"), "CodeTypeMissedCall"),
    (("CodeType", "Sms"), "CodeTypeSms"),
    (("Contacts", "NotModified"), "ContactsNotModified"),
    (("CountriesList", "NotModified"), "CountriesListNotModified"),
    (("DeepLinkInfo", "Empty"), "DeepLinkInfoEmpty"),
    (
        ("DestroyAuthKeyRes", "DestroyAuthKeyFail"),
        "DestroyAuthKeyFail",
    ),
    (
        ("DestroyAuthKeyRes", "DestroyAuthKeyNone"),
        "DestroyAuthKeyNone",
    ),
    (
        ("DestroyAuthKeyRes", "DestroyAuthKeyOk"),
        "DestroyAuthKeyOk",
    ),
    (("DialogFilter", "Default"), "DialogFilterDefault"),
    (
        ("DocumentAttribute", "Animated"),
        "DocumentAttributeAnimated",
    ),
    (
        ("DocumentAttribute", "HasStickers"),
        "DocumentAttributeHasStickers",
    ),
    (
        ("EmailVerifyPurpose", "LoginChange"),
        "EmailVerifyPurposeLoginChange",
    ),
    (
        ("EmailVerifyPurpose", "Passport"),
        "EmailVerifyPurposePassport",
    ),
    (
        ("EmojiGameInfo", "EmojiGameUnavailable"),
        "EmojiGameUnavailable",
    ),
    (("EmojiGroups", "NotModified"), "EmojiGroupsNotModified"),
    (("EmojiList", "NotModified"), "EmojiListNotModified"),
    (("EmojiStatus", "Empty"), "EmojiStatusEmpty"),
    (("EmojiStatuses", "NotModified"), "EmojiStatusesNotModified"),
    (("EncryptedFile", "Empty"), "EncryptedFileEmpty"),
    (
        ("ExportedChatInvite", "ChatInvitePublicJoinRequests"),
        "ChatInvitePublicJoinRequests",
    ),
    (("FavedStickers", "NotModified"), "FavedStickersNotModified"),
    (("FileType", "FileGif"), "FileGif"),
    (("FileType", "FileJpeg"), "FileJpeg"),
    (("FileType", "FileMov"), "FileMov"),
    (("FileType", "FileMp3"), "FileMp3"),
    (("FileType", "FileMp4"), "FileMp4"),
    (("FileType", "FilePartial"), "FilePartial"),
    (("FileType", "FilePdf"), "FilePdf"),
    (("FileType", "FilePng"), "FilePng"),
    (("FileType", "FileUnknown"), "FileUnknown"),
    (("FileType", "FileWebp"), "FileWebp"),
    (
        ("FoundStickerSets", "NotModified"),
        "FoundStickerSetsNotModified",
    ),
    (("GeoPoint", "Empty"), "GeoPointEmpty"),
    (("InlineButtonType", "Buy"), "InlineButtonTypeBuy"),
    (("InlineButtonType", "Disabled"), "InlineButtonTypeDisabled"),
    (("InlineButtonType", "Game"), "InlineButtonTypeGame"),
    (("InlineQueryPeerType", "BotPm"), "InlineQueryPeerTypeBotPm"),
    (
        ("InlineQueryPeerType", "Broadcast"),
        "InlineQueryPeerTypeBroadcast",
    ),
    (("InlineQueryPeerType", "Chat"), "InlineQueryPeerTypeChat"),
    (
        ("InlineQueryPeerType", "Megagroup"),
        "InlineQueryPeerTypeMegagroup",
    ),
    (("InlineQueryPeerType", "Pm"), "InlineQueryPeerTypePm"),
    (
        ("InlineQueryPeerType", "SameBotPm"),
        "InlineQueryPeerTypeSameBotPm",
    ),
    (("InputChannel", "Empty"), "InputChannelEmpty"),
    (("InputChatPhoto", "Empty"), "InputChatPhotoEmpty"),
    (("InputChatTheme", "Empty"), "InputChatThemeEmpty"),
    (
        ("InputCheckPasswordSrp", "InputCheckPasswordEmpty"),
        "InputCheckPasswordEmpty",
    ),
    (("InputDocument", "Empty"), "InputDocumentEmpty"),
    (("InputEncryptedFile", "Empty"), "InputEncryptedFileEmpty"),
    (
        ("InputFileLocation", "InputTakeoutFileLocation"),
        "InputTakeoutFileLocation",
    ),
    (("InputGeoPoint", "Empty"), "InputGeoPointEmpty"),
    (("InputMedia", "Empty"), "InputMediaEmpty"),
    (("InputMessage", "Pinned"), "InputMessagePinned"),
    (
        ("InputNotifyPeer", "InputNotifyBroadcasts"),
        "InputNotifyBroadcasts",
    ),
    (("InputNotifyPeer", "InputNotifyChats"), "InputNotifyChats"),
    (("InputNotifyPeer", "InputNotifyUsers"), "InputNotifyUsers"),
    (("InputPeer", "Empty"), "InputPeerEmpty"),
    (("InputPeer", "PeerSelf"), "InputPeerSelf"),
    (("InputPhoto", "Empty"), "InputPhotoEmpty"),
    (("InputPrivacyKey", "About"), "InputPrivacyKeyAbout"),
    (
        ("InputPrivacyKey", "AddedByPhone"),
        "InputPrivacyKeyAddedByPhone",
    ),
    (("InputPrivacyKey", "Birthday"), "InputPrivacyKeyBirthday"),
    (
        ("InputPrivacyKey", "ChatInvite"),
        "InputPrivacyKeyChatInvite",
    ),
    (("InputPrivacyKey", "Forwards"), "InputPrivacyKeyForwards"),
    (
        ("InputPrivacyKey", "NoPaidMessages"),
        "InputPrivacyKeyNoPaidMessages",
    ),
    (("InputPrivacyKey", "PhoneCall"), "InputPrivacyKeyPhoneCall"),
    (
        ("InputPrivacyKey", "PhoneNumber"),
        "InputPrivacyKeyPhoneNumber",
    ),
    (("InputPrivacyKey", "PhoneP2P"), "InputPrivacyKeyPhoneP2P"),
    (
        ("InputPrivacyKey", "ProfilePhoto"),
        "InputPrivacyKeyProfilePhoto",
    ),
    (
        ("InputPrivacyKey", "SavedMusic"),
        "InputPrivacyKeySavedMusic",
    ),
    (
        ("InputPrivacyKey", "StarGiftsAutoSave"),
        "InputPrivacyKeyStarGiftsAutoSave",
    ),
    (
        ("InputPrivacyKey", "StatusTimestamp"),
        "InputPrivacyKeyStatusTimestamp",
    ),
    (
        ("InputPrivacyKey", "VoiceMessages"),
        "InputPrivacyKeyVoiceMessages",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueAllowAll"),
        "InputPrivacyValueAllowAll",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueAllowBots"),
        "InputPrivacyValueAllowBots",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueAllowCloseFriends"),
        "InputPrivacyValueAllowCloseFriends",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueAllowContacts"),
        "InputPrivacyValueAllowContacts",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueAllowPremium"),
        "InputPrivacyValueAllowPremium",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueDisallowAll"),
        "InputPrivacyValueDisallowAll",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueDisallowBots"),
        "InputPrivacyValueDisallowBots",
    ),
    (
        ("InputPrivacyRule", "InputPrivacyValueDisallowContacts"),
        "InputPrivacyValueDisallowContacts",
    ),
    (
        ("InputStickerSet", "AnimatedEmoji"),
        "InputStickerSetAnimatedEmoji",
    ),
    (
        ("InputStickerSet", "AnimatedEmojiAnimations"),
        "InputStickerSetAnimatedEmojiAnimations",
    ),
    (
        ("InputStickerSet", "EmojiChannelDefaultStatuses"),
        "InputStickerSetEmojiChannelDefaultStatuses",
    ),
    (
        ("InputStickerSet", "EmojiDefaultStatuses"),
        "InputStickerSetEmojiDefaultStatuses",
    ),
    (
        ("InputStickerSet", "EmojiDefaultTopicIcons"),
        "InputStickerSetEmojiDefaultTopicIcons",
    ),
    (
        ("InputStickerSet", "EmojiGenericAnimations"),
        "InputStickerSetEmojiGenericAnimations",
    ),
    (("InputStickerSet", "Empty"), "InputStickerSetEmpty"),
    (
        ("InputStickerSet", "PremiumGifts"),
        "InputStickerSetPremiumGifts",
    ),
    (("InputStickerSet", "TonGifts"), "InputStickerSetTonGifts"),
    (("InputUser", "Empty"), "InputUserEmpty"),
    (("InputUser", "UserSelf"), "InputUserSelf"),
    (
        ("InputWalletReplacement", "InputWalletNew"),
        "InputWalletNew",
    ),
    (
        ("JoinChatBotResult", "Approved"),
        "JoinChatBotResultApproved",
    ),
    (
        ("JoinChatBotResult", "Declined"),
        "JoinChatBotResultDeclined",
    ),
    (("JoinChatBotResult", "Queued"), "JoinChatBotResultQueued"),
    (("Jsonvalue", "JsonNull"), "JsonNull"),
    (
        ("MessageAction", "ChatDeletePhoto"),
        "MessageActionChatDeletePhoto",
    ),
    (
        ("MessageAction", "ChatJoinedByRequest"),
        "MessageActionChatJoinedByRequest",
    ),
    (
        ("MessageAction", "ContactSignUp"),
        "MessageActionContactSignUp",
    ),
    (("MessageAction", "Empty"), "MessageActionEmpty"),
    (
        ("MessageAction", "HistoryClear"),
        "MessageActionHistoryClear",
    ),
    (
        ("MessageAction", "NoPaidMessagesTokenUsed"),
        "MessageActionNoPaidMessagesTokenUsed",
    ),
    (("MessageAction", "PinMessage"), "MessageActionPinMessage"),
    (
        ("MessageAction", "ScreenshotTaken"),
        "MessageActionScreenshotTaken",
    ),
    (("MessageMedia", "Empty"), "MessageMediaEmpty"),
    (("MessageMedia", "Unsupported"), "MessageMediaUnsupported"),
    (
        ("MessagesFilter", "InputMessagesFilterChatPhotos"),
        "InputMessagesFilterChatPhotos",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterContacts"),
        "InputMessagesFilterContacts",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterDocument"),
        "InputMessagesFilterDocument",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterEmpty"),
        "InputMessagesFilterEmpty",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterGeo"),
        "InputMessagesFilterGeo",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterGif"),
        "InputMessagesFilterGif",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterMusic"),
        "InputMessagesFilterMusic",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterMyMentions"),
        "InputMessagesFilterMyMentions",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterPhotoVideo"),
        "InputMessagesFilterPhotoVideo",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterPhotos"),
        "InputMessagesFilterPhotos",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterPinned"),
        "InputMessagesFilterPinned",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterPoll"),
        "InputMessagesFilterPoll",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterRoundVideo"),
        "InputMessagesFilterRoundVideo",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterRoundVoice"),
        "InputMessagesFilterRoundVoice",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterUrl"),
        "InputMessagesFilterUrl",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterVideo"),
        "InputMessagesFilterVideo",
    ),
    (
        ("MessagesFilter", "InputMessagesFilterVoice"),
        "InputMessagesFilterVoice",
    ),
    (("NotificationSound", "Default"), "NotificationSoundDefault"),
    (("NotificationSound", "None"), "NotificationSoundNone"),
    (("NotifyPeer", "NotifyBroadcasts"), "NotifyBroadcasts"),
];
