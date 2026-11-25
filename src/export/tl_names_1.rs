//! Native Grammers 02367f97 constructor-name data (MIT OR Apache-2.0).
pub(super) const UNITS: &[((&str, &str), &str)] = &[
    (("NotifyPeer", "NotifyChats"), "NotifyChats"),
    (("NotifyPeer", "NotifyUsers"), "NotifyUsers"),
    (("Null", "Null"), "Null"),
    (("PageBlock", "Divider"), "PageBlockDivider"),
    (("PageBlock", "Unsupported"), "PageBlockUnsupported"),
    (
        ("PaidReactionPrivacy", "Anonymous"),
        "PaidReactionPrivacyAnonymous",
    ),
    (
        ("PaidReactionPrivacy", "Default"),
        "PaidReactionPrivacyDefault",
    ),
    (
        ("PassportConfig", "NotModified"),
        "PassportConfigNotModified",
    ),
    (("PasswordKdfAlgo", "Unknown"), "PasswordKdfAlgoUnknown"),
    (("PeerColors", "NotModified"), "PeerColorsNotModified"),
    (
        ("PhoneCallDiscardReason", "Busy"),
        "PhoneCallDiscardReasonBusy",
    ),
    (
        ("PhoneCallDiscardReason", "Disconnect"),
        "PhoneCallDiscardReasonDisconnect",
    ),
    (
        ("PhoneCallDiscardReason", "Hangup"),
        "PhoneCallDiscardReasonHangup",
    ),
    (
        ("PhoneCallDiscardReason", "Missed"),
        "PhoneCallDiscardReasonMissed",
    ),
    (("PrivacyKey", "About"), "PrivacyKeyAbout"),
    (("PrivacyKey", "AddedByPhone"), "PrivacyKeyAddedByPhone"),
    (("PrivacyKey", "Birthday"), "PrivacyKeyBirthday"),
    (("PrivacyKey", "ChatInvite"), "PrivacyKeyChatInvite"),
    (("PrivacyKey", "Forwards"), "PrivacyKeyForwards"),
    (("PrivacyKey", "NoPaidMessages"), "PrivacyKeyNoPaidMessages"),
    (("PrivacyKey", "PhoneCall"), "PrivacyKeyPhoneCall"),
    (("PrivacyKey", "PhoneNumber"), "PrivacyKeyPhoneNumber"),
    (("PrivacyKey", "PhoneP2P"), "PrivacyKeyPhoneP2P"),
    (("PrivacyKey", "ProfilePhoto"), "PrivacyKeyProfilePhoto"),
    (("PrivacyKey", "SavedMusic"), "PrivacyKeySavedMusic"),
    (
        ("PrivacyKey", "StarGiftsAutoSave"),
        "PrivacyKeyStarGiftsAutoSave",
    ),
    (
        ("PrivacyKey", "StatusTimestamp"),
        "PrivacyKeyStatusTimestamp",
    ),
    (("PrivacyKey", "VoiceMessages"), "PrivacyKeyVoiceMessages"),
    (
        ("PrivacyRule", "PrivacyValueAllowAll"),
        "PrivacyValueAllowAll",
    ),
    (
        ("PrivacyRule", "PrivacyValueAllowBots"),
        "PrivacyValueAllowBots",
    ),
    (
        ("PrivacyRule", "PrivacyValueAllowCloseFriends"),
        "PrivacyValueAllowCloseFriends",
    ),
    (
        ("PrivacyRule", "PrivacyValueAllowContacts"),
        "PrivacyValueAllowContacts",
    ),
    (
        ("PrivacyRule", "PrivacyValueAllowPremium"),
        "PrivacyValueAllowPremium",
    ),
    (
        ("PrivacyRule", "PrivacyValueDisallowAll"),
        "PrivacyValueDisallowAll",
    ),
    (
        ("PrivacyRule", "PrivacyValueDisallowBots"),
        "PrivacyValueDisallowBots",
    ),
    (
        ("PrivacyRule", "PrivacyValueDisallowContacts"),
        "PrivacyValueDisallowContacts",
    ),
    (("ProfileTab", "Files"), "ProfileTabFiles"),
    (("ProfileTab", "Gifs"), "ProfileTabGifs"),
    (("ProfileTab", "Gifts"), "ProfileTabGifts"),
    (("ProfileTab", "Links"), "ProfileTabLinks"),
    (("ProfileTab", "Media"), "ProfileTabMedia"),
    (("ProfileTab", "Music"), "ProfileTabMusic"),
    (("ProfileTab", "Posts"), "ProfileTabPosts"),
    (("ProfileTab", "Voice"), "ProfileTabVoice"),
    (("QuickReplies", "NotModified"), "QuickRepliesNotModified"),
    (("Reaction", "Empty"), "ReactionEmpty"),
    (("Reaction", "Paid"), "ReactionPaid"),
    (
        ("ReactionNotificationsFrom", "All"),
        "ReactionNotificationsFromAll",
    ),
    (
        ("ReactionNotificationsFrom", "Contacts"),
        "ReactionNotificationsFromContacts",
    ),
    (("Reactions", "NotModified"), "ReactionsNotModified"),
    (
        ("RecentStickers", "NotModified"),
        "RecentStickersNotModified",
    ),
    (
        ("ReportReason", "InputReportReasonChildAbuse"),
        "InputReportReasonChildAbuse",
    ),
    (
        ("ReportReason", "InputReportReasonCopyright"),
        "InputReportReasonCopyright",
    ),
    (
        ("ReportReason", "InputReportReasonFake"),
        "InputReportReasonFake",
    ),
    (
        ("ReportReason", "InputReportReasonGeoIrrelevant"),
        "InputReportReasonGeoIrrelevant",
    ),
    (
        ("ReportReason", "InputReportReasonIllegalDrugs"),
        "InputReportReasonIllegalDrugs",
    ),
    (
        ("ReportReason", "InputReportReasonOther"),
        "InputReportReasonOther",
    ),
    (
        ("ReportReason", "InputReportReasonPersonalDetails"),
        "InputReportReasonPersonalDetails",
    ),
    (
        ("ReportReason", "InputReportReasonPornography"),
        "InputReportReasonPornography",
    ),
    (
        ("ReportReason", "InputReportReasonSpam"),
        "InputReportReasonSpam",
    ),
    (
        ("ReportReason", "InputReportReasonViolence"),
        "InputReportReasonViolence",
    ),
    (("ReportResult", "Reported"), "ReportResultReported"),
    (
        ("RequirementToContact", "Empty"),
        "RequirementToContactEmpty",
    ),
    (
        ("RequirementToContact", "Premium"),
        "RequirementToContactPremium",
    ),
    (
        ("ResetPasswordResult", "ResetPasswordOk"),
        "ResetPasswordOk",
    ),
    (("RichText", "TextEmpty"), "TextEmpty"),
    (
        ("RpcDropAnswer", "RpcAnswerDroppedRunning"),
        "RpcAnswerDroppedRunning",
    ),
    (("RpcDropAnswer", "RpcAnswerUnknown"), "RpcAnswerUnknown"),
    (("SavedGifs", "NotModified"), "SavedGifsNotModified"),
    (("SavedMusicIds", "NotModified"), "SavedMusicIdsNotModified"),
    (
        ("SavedReactionTags", "NotModified"),
        "SavedReactionTagsNotModified",
    ),
    (("SavedRingtone", "Ringtone"), "SavedRingtone"),
    (
        ("SavedRingtones", "NotModified"),
        "SavedRingtonesNotModified",
    ),
    (("SecureFile", "Empty"), "SecureFileEmpty"),
    (
        ("SecurePasswordKdfAlgo", "Unknown"),
        "SecurePasswordKdfAlgoUnknown",
    ),
    (("SecureValueType", "Address"), "SecureValueTypeAddress"),
    (
        ("SecureValueType", "BankStatement"),
        "SecureValueTypeBankStatement",
    ),
    (
        ("SecureValueType", "DriverLicense"),
        "SecureValueTypeDriverLicense",
    ),
    (("SecureValueType", "Email"), "SecureValueTypeEmail"),
    (
        ("SecureValueType", "IdentityCard"),
        "SecureValueTypeIdentityCard",
    ),
    (
        ("SecureValueType", "InternalPassport"),
        "SecureValueTypeInternalPassport",
    ),
    (("SecureValueType", "Passport"), "SecureValueTypePassport"),
    (
        ("SecureValueType", "PassportRegistration"),
        "SecureValueTypePassportRegistration",
    ),
    (
        ("SecureValueType", "PersonalDetails"),
        "SecureValueTypePersonalDetails",
    ),
    (("SecureValueType", "Phone"), "SecureValueTypePhone"),
    (
        ("SecureValueType", "RentalAgreement"),
        "SecureValueTypeRentalAgreement",
    ),
    (
        ("SecureValueType", "TemporaryRegistration"),
        "SecureValueTypeTemporaryRegistration",
    ),
    (
        ("SecureValueType", "UtilityBill"),
        "SecureValueTypeUtilityBill",
    ),
    (
        ("SendMessageAction", "SendMessageCancelAction"),
        "SendMessageCancelAction",
    ),
    (
        ("SendMessageAction", "SendMessageChooseContactAction"),
        "SendMessageChooseContactAction",
    ),
    (
        ("SendMessageAction", "SendMessageChooseStickerAction"),
        "SendMessageChooseStickerAction",
    ),
    (
        ("SendMessageAction", "SendMessageGamePlayAction"),
        "SendMessageGamePlayAction",
    ),
    (
        ("SendMessageAction", "SendMessageGeoLocationAction"),
        "SendMessageGeoLocationAction",
    ),
    (
        ("SendMessageAction", "SendMessageRecordAudioAction"),
        "SendMessageRecordAudioAction",
    ),
    (
        ("SendMessageAction", "SendMessageRecordRoundAction"),
        "SendMessageRecordRoundAction",
    ),
    (
        ("SendMessageAction", "SendMessageRecordVideoAction"),
        "SendMessageRecordVideoAction",
    ),
    (
        ("SendMessageAction", "SendMessageTypingAction"),
        "SendMessageTypingAction",
    ),
    (
        ("SendMessageAction", "SpeakingInGroupCallAction"),
        "SpeakingInGroupCallAction",
    ),
    (("ShareTheme", "Default"), "ShareThemeDefault"),
    (
        ("ShareThemeBackdrops", "NotModified"),
        "ShareThemeBackdropsNotModified",
    ),
    (
        ("SponsoredMessageReportResult", "AdsHidden"),
        "SponsoredMessageReportResultAdsHidden",
    ),
    (
        ("SponsoredMessageReportResult", "Reported"),
        "SponsoredMessageReportResultReported",
    ),
    (("SponsoredMessages", "Empty"), "SponsoredMessagesEmpty"),
    (("SponsoredPeers", "Empty"), "SponsoredPeersEmpty"),
    (
        ("StarGiftActiveAuctions", "NotModified"),
        "StarGiftActiveAuctionsNotModified",
    ),
    (
        ("StarGiftAttributeRarity", "Epic"),
        "StarGiftAttributeRarityEpic",
    ),
    (
        ("StarGiftAttributeRarity", "Legendary"),
        "StarGiftAttributeRarityLegendary",
    ),
    (
        ("StarGiftAttributeRarity", "Rare"),
        "StarGiftAttributeRarityRare",
    ),
    (
        ("StarGiftAttributeRarity", "Uncommon"),
        "StarGiftAttributeRarityUncommon",
    ),
    (
        ("StarGiftAuctionState", "NotModified"),
        "StarGiftAuctionStateNotModified",
    ),
    (
        ("StarGiftCollections", "NotModified"),
        "StarGiftCollectionsNotModified",
    ),
    (("StarGifts", "NotModified"), "StarGiftsNotModified"),
    (("StarsTransactionPeer", "Ads"), "StarsTransactionPeerAds"),
    (("StarsTransactionPeer", "Api"), "StarsTransactionPeerApi"),
    (
        ("StarsTransactionPeer", "AppStore"),
        "StarsTransactionPeerAppStore",
    ),
    (
        ("StarsTransactionPeer", "Fragment"),
        "StarsTransactionPeerFragment",
    ),
    (
        ("StarsTransactionPeer", "PlayMarket"),
        "StarsTransactionPeerPlayMarket",
    ),
    (
        ("StarsTransactionPeer", "PremiumBot"),
        "StarsTransactionPeerPremiumBot",
    ),
    (
        ("StarsTransactionPeer", "Unsupported"),
        "StarsTransactionPeerUnsupported",
    ),
    (("StickerSet", "NotModified"), "StickerSetNotModified"),
    (
        ("StickerSetInstallResult", "Success"),
        "StickerSetInstallResultSuccess",
    ),
    (("Stickers", "NotModified"), "StickersNotModified"),
    (("Themes", "NotModified"), "ThemesNotModified"),
    (("TimezonesList", "NotModified"), "TimezonesListNotModified"),
    (("TlsBlock", "Domain"), "TlsBlockDomain"),
    (("TlsBlock", "PublicKey"), "TlsBlockPublicKey"),
    (("Tones", "NotModified"), "TonesNotModified"),
    (("TopPeerCategory", "BotsApp"), "TopPeerCategoryBotsApp"),
    (
        ("TopPeerCategory", "BotsGuestChat"),
        "TopPeerCategoryBotsGuestChat",
    ),
    (
        ("TopPeerCategory", "BotsInline"),
        "TopPeerCategoryBotsInline",
    ),
    (("TopPeerCategory", "BotsPm"), "TopPeerCategoryBotsPm"),
    (("TopPeerCategory", "Channels"), "TopPeerCategoryChannels"),
    (
        ("TopPeerCategory", "Correspondents"),
        "TopPeerCategoryCorrespondents",
    ),
    (
        ("TopPeerCategory", "ForwardChats"),
        "TopPeerCategoryForwardChats",
    ),
    (
        ("TopPeerCategory", "ForwardUsers"),
        "TopPeerCategoryForwardUsers",
    ),
    (("TopPeerCategory", "Groups"), "TopPeerCategoryGroups"),
    (
        ("TopPeerCategory", "PhoneCalls"),
        "TopPeerCategoryPhoneCalls",
    ),
    (("TopPeers", "Disabled"), "TopPeersDisabled"),
    (("TopPeers", "NotModified"), "TopPeersNotModified"),
    (("True", "True"), "True"),
    (("Update", "AiComposeTones"), "UpdateAiComposeTones"),
    (("Update", "AttachMenuBots"), "UpdateAttachMenuBots"),
    (("Update", "AutoSaveSettings"), "UpdateAutoSaveSettings"),
    (("Update", "Config"), "UpdateConfig"),
    (("Update", "ContactsReset"), "UpdateContactsReset"),
    (("Update", "DialogFilters"), "UpdateDialogFilters"),
    (("Update", "FavedStickers"), "UpdateFavedStickers"),
    (("Update", "LoginToken"), "UpdateLoginToken"),
    (("Update", "PtsChanged"), "UpdatePtsChanged"),
    (
        ("Update", "ReadFeaturedEmojiStickers"),
        "UpdateReadFeaturedEmojiStickers",
    ),
    (
        ("Update", "ReadFeaturedStickers"),
        "UpdateReadFeaturedStickers",
    ),
    (
        ("Update", "RecentEmojiStatuses"),
        "UpdateRecentEmojiStatuses",
    ),
    (("Update", "RecentReactions"), "UpdateRecentReactions"),
    (("Update", "RecentStickers"), "UpdateRecentStickers"),
    (("Update", "SavedGifs"), "UpdateSavedGifs"),
    (("Update", "SavedReactionTags"), "UpdateSavedReactionTags"),
    (("Update", "SavedRingtones"), "UpdateSavedRingtones"),
    (("Update", "StarGiftCraftFail"), "UpdateStarGiftCraftFail"),
    (("Updates", "TooLong"), "UpdatesTooLong"),
    (("UrlAuthResult", "Default"), "UrlAuthResultDefault"),
    (("UserInfo", "Empty"), "UserInfoEmpty"),
    (("UserProfilePhoto", "Empty"), "UserProfilePhotoEmpty"),
    (("UserStatus", "Empty"), "UserStatusEmpty"),
    (("WallPapers", "NotModified"), "WallPapersNotModified"),
    (
        ("WalletOnchainGiftAction", "Buy"),
        "WalletOnchainGiftActionBuy",
    ),
    (
        ("WalletOnchainGiftAction", "Delist"),
        "WalletOnchainGiftActionDelist",
    ),
    (
        ("WalletTransactionPeer", "Unsupported"),
        "WalletTransactionPeerUnsupported",
    ),
    (
        ("WebBrowserSettings", "NotModified"),
        "WebBrowserSettingsNotModified",
    ),
    (
        ("WelcomeMessages", "NotModified"),
        "WelcomeMessagesNotModified",
    ),
];
