# API inventory

Snapshot: 2026-10-25, current checkout. The vendored manifest pins `evex-dev/linejs` to v3.5.0. This inventory describes the local source, not an official LINE API support guarantee.

There are **46 app RPC methods**, **39 exported service classes** with **430 public named method declarations**, plus Timeline and object-storage helpers listed below. The service count includes local helpers and repeated method names across classes; it is not a count of distinct server endpoints.

## App RPC methods

These methods are dispatched by [`protocol/src/main.ts`](../protocol/src/main.ts). They run over the local JSON-lines sidecar connection, not HTTP routes. Wired dispatch does not imply every operation has been verified against a live account.

| Area | Methods |
| --- | --- |
| Session | `login_qr`, `login_email`, `login_token`, `logout` |
| Chats and contacts | `list_chats`, `get_chat_members`, `list_friends`, `add_friend`, `get_user_profile`, `get_blocked_contacts`, `set_contact_blocked`, `mute_chat` |
| Message history | `load_cached_messages`, `fetch_messages`, `fetch_older_messages`, `fetch_newer_messages` |
| Sending and removing messages | `send_message`, `send_audio`, `send_media`, `send_sticker`, `send_postback`, `unsend_message` |
| Reactions | `react_message`, `cancel_reaction`, `reaction_emojis`, `reaction_emoji_packs`, `reaction_emoji_pack` |
| Media and stickers | `download_media`, `list_stickers` |
| Shared media | `list_shared_media`, `download_shared_media`, `stop_shared_media`, `fetch_shared_message_context` |
| Albums | `list_albums`, `list_album_photos`, `download_album_photo` |
| Read receipts | `mark_read`, `mark_read_once` |
| Calls | `call_start`, `call_answer`, `call_decline`, `call_end`, `call_set_audio` |
| Utilities | `ping`, `set_language`, `clear_cache` |

Request envelope:

```json
{"id":1,"method":"send_message","params":{"chatMid":"c-example","text":"Hello"}}
```

Responses use the request ID, `ok`, and `result` or `error`. Asynchronous events use `event` plus event-specific fields. Consult each dispatch branch and its service for exact parameters and results.

Important boundaries:

- Album RPCs currently list and download; there are no app RPCs for creating or changing albums.
- There is no wired native poll creation, poll voting, reaction-poll tallying, or scheduler RPC.
- `food_poll_read`, `food_poll_save`, and `food_poll_preview` appear in the disconnected food preparation UI, but are not dispatched by `main.ts`.
- `mute_chat` currently accepts direct-message conversations only.
- `send_postback` can fall back to sending a normal chat message.

## Vendored LINE service wrappers

These classes are exported from [`base/service/mod.ts`](../protocol/vendor/deps/linejs/base/service/mod.ts). The default BaseClient directly initializes `auth`, `call`, `channel`, `liff`, `moa`, `relation`, `livetalk`, `square`, and `talk`; the other exported classes need additional integration. Methods below are public named declarations in each class file; inherited methods and constructors are excluded. Method availability, permissions, authentication, and live behavior depend on the service and account.

| Service class | Declared methods |
| --- | ---: |
| [AccessTokenRefreshService](#accesstokenrefreshservice) | 2 |
| [AccountAuthFactorEapConnectService](#accountauthfactoreapconnectservice) | 4 |
| [AuthService](#authservice) | 19 |
| [BotExternalService](#botexternalservice) | 2 |
| [BuddyService](#buddyservice) | 4 |
| [CallService](#callservice) | 18 |
| [ChannelService](#channelservice) | 12 |
| [ChatAppService](#chatappservice) | 2 |
| [CoinService](#coinservice) | 5 |
| [DeviceAttestationService](#deviceattestationservice) | 4 |
| [E2EEKeyBackupService](#e2eekeybackupservice) | 2 |
| [HomeSafetyCheckService](#homesafetycheckservice) | 3 |
| [LiffService](#liffservice) | 5 |
| [MoaService](#moaservice) | 5 |
| [MultiProfileService](#multiprofileservice) | 7 |
| [OaChatService](#oachatservice) | 1 |
| [OaMembershipService](#oamembershipservice) | 2 |
| [PasswordUpdateService](#passwordupdateservice) | 4 |
| [PremiumFontService](#premiumfontservice) | 2 |
| [PremiumStatusService](#premiumstatusservice) | 4 |
| [PrimaryAccountInitService](#primaryaccountinitservice) | 16 |
| [PrimaryAccountReLoginService](#primaryaccountreloginservice) | 6 |
| [PrimaryAccountSmartSwitchRestorePreparationService](#primaryaccountsmartswitchrestorepreparationservice) | 1 |
| [PrimaryAccountSmartSwitchRestoreService](#primaryaccountsmartswitchrestoreservice) | 14 |
| [PrimaryQrCodeMigrationLongPollingService](#primaryqrcodemigrationlongpollingservice) | 1 |
| [PrimaryQrCodeMigrationPreparationService](#primaryqrcodemigrationpreparationservice) | 2 |
| [PrimarySeamlessLoginService](#primaryseamlessloginservice) | 1 |
| [PwlessPrimaryRegistrationService](#pwlessprimaryregistrationservice) | 2 |
| [RelationService](#relationservice) | 11 |
| [SecondaryPwlessLoginService](#secondarypwlessloginservice) | 1 |
| [SecondarySeamlessLoginService](#secondaryseamlessloginservice) | 1 |
| [SettingsService](#settingsservice) | 2 |
| [ShopAuthService](#shopauthservice) | 1 |
| [ShopCollectionService](#shopcollectionservice) | 5 |
| [ShopService](#shopservice) | 29 |
| [SquareBotService](#squarebotservice) | 1 |
| [SquareLiveTalkService](#squarelivetalkservice) | 26 |
| [SquareService](#squareservice) | 84 |
| [TalkService](#talkservice) | 119 |

## AccessTokenRefreshService

Source: [`protocol/vendor/deps/linejs/base/service/accesstokenrefresh/mod.ts`](../protocol/vendor/deps/linejs/base/service/accesstokenrefresh/mod.ts).

Declared request path: `/EXT/auth/tokenrefresh/v1`.

- `refresh`
- `reportRefreshedAccessToken`

## AccountAuthFactorEapConnectService

Source: [`protocol/vendor/deps/linejs/base/service/accountauthfactoreapconnect/mod.ts`](../protocol/vendor/deps/linejs/base/service/accountauthfactoreapconnect/mod.ts).

Declared request path: `/ACCT/authfactor/eap/v1`.

- `connectEapAccount`
- `disconnectEapAccount`
- `openSession`
- `verifyEapLogin`

## AuthService

Source: [`protocol/vendor/deps/linejs/base/service/auth/mod.ts`](../protocol/vendor/deps/linejs/base/service/auth/mod.ts).

Declared request path: `/AS4`.

- `tryRefreshToken`
- `refresh`
- `reportRefreshedAccessToken`
- `connectEapAccount`
- `disconnectEapAccount`
- `openSession`
- `verifyEapLogin`
- `updatePassword`
- `establishE2EESession`
- `issueTokenForAccountMigrationSettings`
- `openAuthSession`
- `getAuthRSAKey`
- `setIdentifier`
- `updateIdentifier`
- `resendIdentifierConfirmation`
- `confirmIdentifier`
- `removeIdentifier`
- `issueV3TokenForPrimary`
- `logoutZ`

## BotExternalService

Source: [`protocol/vendor/deps/linejs/base/service/botexternal/mod.ts`](../protocol/vendor/deps/linejs/base/service/botexternal/mod.ts).

Declared request path: `/BOTE`.

- `notifyOATalkroomEvents`
- `notifyChatAdEntry`

## BuddyService

Source: [`protocol/vendor/deps/linejs/base/service/buddy/mod.ts`](../protocol/vendor/deps/linejs/base/service/buddy/mod.ts).

Declared request path: `/BUDDY4`.

- `getPromotedBuddyContacts`
- `getBuddyDetailWithPersonal`
- `getNewlyReleasedBuddyIds`
- `getBuddyDetail`

## CallService

Source: [`protocol/vendor/deps/linejs/base/service/call/mod.ts`](../protocol/vendor/deps/linejs/base/service/call/mod.ts).

Declared request path: `/V4`.

- `acquireCallRoute`
- `acquireOACallRoute`
- `lookupPaidCall`
- `acquirePaidCallRoute`
- `acquireGroupCallRoute`
- `getGroupCall`
- `inviteIntoGroupCall`
- `getGroupCallUrls`
- `createGroupCallUrl`
- `deleteGroupCallUrl`
- `updateGroupCallUrl`
- `getGroupCallUrlInfo`
- `joinChatByCallUrl`
- `kickoutFromGroupCall`
- `startPhotobooth`
- `usePhotoboothTicket`
- `getPhotoboothBalance`
- `getCallStatus`

## ChannelService

Source: [`protocol/vendor/deps/linejs/base/service/channel/mod.ts`](../protocol/vendor/deps/linejs/base/service/channel/mod.ts).

Declared request path: `/CH4`.

- `approveChannelAndIssueChannelToken`
- `getChannelInfo`
- `getCommonDomains`
- `issueRequestTokenWithAuthScheme`
- `getReturnUrlWithRequestTokenForAutoLogin`
- `getWebLoginDisallowedUrl`
- `updateChannelNotificationSetting`
- `updateChannelSettings`
- `getUpdatedChannelIds`
- `getChannelNotificationSettings`
- `getApprovedChannels`
- `issueChannelToken`

## ChatAppService

Source: [`protocol/vendor/deps/linejs/base/service/chatapp/mod.ts`](../protocol/vendor/deps/linejs/base/service/chatapp/mod.ts).

Declared request path: `/CAPP1`.

- `getChatapp`
- `getMyChatapps`

## CoinService

Source: [`protocol/vendor/deps/linejs/base/service/coin/mod.ts`](../protocol/vendor/deps/linejs/base/service/coin/mod.ts).

Declared request path: `/COIN4`.

- `getTotalCoinBalance`
- `getCoinPurchaseHistory`
- `getCoinProducts`
- `reserveCoinPurchase`
- `getCoinUseAndRefundHistory`

## DeviceAttestationService

Source: [`protocol/vendor/deps/linejs/base/service/deviceattestation/mod.ts`](../protocol/vendor/deps/linejs/base/service/deviceattestation/mod.ts).

Declared request path: `/EXT/auth/feature-user/api/primary/device-attestation`.

- `getAssertionChallenge`
- `getAttestationChallenge`
- `verifyAssertion`
- `verifyAttestation`

## E2EEKeyBackupService

Source: [`protocol/vendor/deps/linejs/base/service/e2eekeybackup/mod.ts`](../protocol/vendor/deps/linejs/base/service/e2eekeybackup/mod.ts).

Declared request path: `/EKBS4`.

- `getE2EEKeyBackupCertificates`
- `getE2EEKeyBackupInfo`

## HomeSafetyCheckService

Source: [`protocol/vendor/deps/linejs/base/service/homesafetycheck/mod.ts`](../protocol/vendor/deps/linejs/base/service/homesafetycheck/mod.ts).

Declared request path: `/EXT/home/safety-check/safety-check`.

- `deleteSafetyStatus`
- `getDisasterCases`
- `updateSafetyStatus`

## LiffService

Source: [`protocol/vendor/deps/linejs/base/service/liff/mod.ts`](../protocol/vendor/deps/linejs/base/service/liff/mod.ts).

Declared request path: `/LIFF1`.

- `issueLiffView`
- `getLiffViewWithoutUserContext`
- `issueSubLiffView`
- `getLiffToken`
- `sendLiff`

## MoaService

Source: [`protocol/vendor/deps/linejs/base/service/moa/mod.ts`](../protocol/vendor/deps/linejs/base/service/moa/mod.ts).

- `clearAlbumChannelToken`
- `getAlbumChannelToken`
- `getAlbums`
- `getPhotos`
- `downloadPhoto`

## MultiProfileService

Source: [`protocol/vendor/deps/linejs/base/service/multiprofile/mod.ts`](../protocol/vendor/deps/linejs/base/service/multiprofile/mod.ts).

Declared request path: `/PFS4`.

- `deleteMultiProfile`
- `getMappedProfileIds`
- `mapProfileToUsers`
- `updateProfileAttributes`
- `createMultiProfile`
- `getProfile`
- `getUsersMappedByProfile`

## OaChatService

Source: [`protocol/vendor/deps/linejs/base/service/oachat/mod.ts`](../protocol/vendor/deps/linejs/base/service/oachat/mod.ts).

Declared request path: `/ext/oachat/api/`.

- `getCallStatus`

## OaMembershipService

Source: [`protocol/vendor/deps/linejs/base/service/oamembership/mod.ts`](../protocol/vendor/deps/linejs/base/service/oamembership/mod.ts).

Declared request path: `/EXT/bot/oafan`.

- `activateSubscription`
- `getJoinedMembership`

## PasswordUpdateService

Source: [`protocol/vendor/deps/linejs/base/service/passwordupdate/mod.ts`](../protocol/vendor/deps/linejs/base/service/passwordupdate/mod.ts).

Declared request path: `/EXT/auth/feature-user/api/primary/password/update`.

- `createSession`
- `getPasswordHashingParameter`
- `setPassword`
- `updatePassword`

## PremiumFontService

Source: [`protocol/vendor/deps/linejs/base/service/premiumfont/mod.ts`](../protocol/vendor/deps/linejs/base/service/premiumfont/mod.ts).

Declared request path: `/EXT/talk/asset-meta-server/thrift/fontService`.

- `getAccessToken`
- `getFontMetas`

## PremiumStatusService

Source: [`protocol/vendor/deps/linejs/base/service/premiumstatus/mod.ts`](../protocol/vendor/deps/linejs/base/service/premiumstatus/mod.ts).

Declared request path: `/EXT/line-premium/common/thrift/status`.

- `getPremiumStatus`
- `getPremiumStatusForUpgrade`
- `getDataRetention`
- `getIncentiveStatus`

## PrimaryAccountInitService

Source: [`protocol/vendor/deps/linejs/base/service/primaryaccountinit/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryaccountinit/mod.ts).

Declared request path: `/acct/pais/v1`.

- `openSession`
- `getCountryInfo`
- `validateProfile`
- `setPassword`
- `getPhoneVerifMethodV2`
- `requestToSendPhonePinCode`
- `verifyPhonePinCode`
- `registerPrimaryUsingPhoneWithTokenV3`
- `lookupAvailableEap`
- `getAllowedRegistrationMethod`
- `verifyEapAccountForRegistration`
- `registerPrimaryUsingEapAccount`
- `getPhoneVerifMethodForRegistration`
- `getAcctVerifMethod`
- `getPasswordHashingParametersForPwdReg`
- `setHashedPassword`

## PrimaryAccountReLoginService

Source: [`protocol/vendor/deps/linejs/base/service/primaryaccountrelogin/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryaccountrelogin/mod.ts).

Declared request path: `/ext/auth/feature-guest/thrift/account/v1/relogin/primary`.

- `createSession`
- `getCountryInfo`
- `getPhoneVerifMethodV2`
- `requestToSendPhonePinCode`
- `verifyEapLogin`
- `verifyPhonePinCode`

## PrimaryAccountSmartSwitchRestorePreparationService

Source: [`protocol/vendor/deps/linejs/base/service/primaryaccountsmartswitchrestorepreparation/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryaccountsmartswitchrestorepreparation/mod.ts).

Declared request path: `/EXT/auth/feature-user/thrift/account/v1/restore/smart-switch/prepare`.

- `createSession`

## PrimaryAccountSmartSwitchRestoreService

Source: [`protocol/vendor/deps/linejs/base/service/primaryaccountsmartswitchrestore/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryaccountsmartswitchrestore/mod.ts).

Declared request path: `/ext/auth/feature-guest/thrift/account/v1/restore/smart-switch`.

- `checkEmailAssigned`
- `checkIfPasswordSetVerificationEmailVerified`
- `getAcctVerifMethod`
- `getCountryInfo`
- `getMaskedEmail`
- `getPasswordHashingParametersForPwdReg`
- `getPasswordHashingParametersForPwdVerif`
- `getPhoneVerifMethodV2`
- `requestToSendPasswordSetVerificationEmail`
- `requestToSendPhonePinCode`
- `setHashedPassword`
- `verifyAccountUsingHashedPwd`
- `verifyEapLogin`
- `verifyPhonePinCode`

## PrimaryQrCodeMigrationLongPollingService

Source: [`protocol/vendor/deps/linejs/base/service/primaryqrcodemigrationlongpolling/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryqrcodemigrationlongpolling/mod.ts).

Declared request path: `/EXT/auth/feature-user/lp/api/primary/mig/qr`.

- `checkIfEncryptedE2EEKeyReceived`

## PrimaryQrCodeMigrationPreparationService

Source: [`protocol/vendor/deps/linejs/base/service/primaryqrcodemigrationpreparation/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryqrcodemigrationpreparation/mod.ts).

Declared request path: `/EXT/auth/feature-user/api/primary/mig/qr/prepare`.

- `createSession`
- `sendEncryptedE2EEKey`

## PrimarySeamlessLoginService

Source: [`protocol/vendor/deps/linejs/base/service/primaryseamlesslogin/mod.ts`](../protocol/vendor/deps/linejs/base/service/primaryseamlesslogin/mod.ts).

Declared request path: `/EXT/auth/feature-user/api/secondary/login/seamless`.

- `permitLogin`

## PwlessPrimaryRegistrationService

Source: [`protocol/vendor/deps/linejs/base/service/pwlessprimaryregistration/mod.ts`](../protocol/vendor/deps/linejs/base/service/pwlessprimaryregistration/mod.ts).

Declared request path: `/ACCT/authfactor/pwless/v1`.

- `createSession`
- `getChallengeForPrimaryReg`

## RelationService

Source: [`protocol/vendor/deps/linejs/base/service/relation/mod.ts`](../protocol/vendor/deps/linejs/base/service/relation/mod.ts).

Declared request path: `/RE4`.

- `getTargetProfiles`
- `getRecommendationDetails`
- `getContactCalendarEvents`
- `getContactsV3`
- `getFriendDetails`
- `getUserFriendIds`
- `addFriendByMid`
- `findContactBySearchIdOrTicketV3`
- `addFriendByUserId`
- `findContactByPhoneV3`
- `addFriendByPhone`

## SecondaryPwlessLoginService

Source: [`protocol/vendor/deps/linejs/base/service/secondarypwlesslogin/mod.ts`](../protocol/vendor/deps/linejs/base/service/secondarypwlesslogin/mod.ts).

Declared request path: `/acct/lgn/secpwless/v1`.

- `createSession`

## SecondarySeamlessLoginService

Source: [`protocol/vendor/deps/linejs/base/service/secondaryseamlesslogin/mod.ts`](../protocol/vendor/deps/linejs/base/service/secondaryseamlesslogin/mod.ts).

Declared request path: `/ext/auth/feature-guest/api/secondary/login/seamless`.

- `createSession`

## SettingsService

Source: [`protocol/vendor/deps/linejs/base/service/settings/mod.ts`](../protocol/vendor/deps/linejs/base/service/settings/mod.ts).

Declared request path: `/US4`.

- `bulkGetSetting`
- `bulkSetSetting`

## ShopAuthService

Source: [`protocol/vendor/deps/linejs/base/service/shopauth/mod.ts`](../protocol/vendor/deps/linejs/base/service/shopauth/mod.ts).

Declared request path: `/SHOPA`.

- `establishE2EESession`

## ShopCollectionService

Source: [`protocol/vendor/deps/linejs/base/service/shopcollection/mod.ts`](../protocol/vendor/deps/linejs/base/service/shopcollection/mod.ts).

Declared request path: `/ext/sapi/sapic`.

- `getUserCollections`
- `createCollectionForUser`
- `addItemToCollection`
- `removeItemFromCollection`
- `isProductForCollections`

## ShopService

Source: [`protocol/vendor/deps/linejs/base/service/shop/mod.ts`](../protocol/vendor/deps/linejs/base/service/shop/mod.ts).

Declared request path: `/TSHOP4`.

- `getProductsByAuthor`
- `getStudentInformation`
- `canReceivePresent`
- `getOwnedProductSummaries`
- `getProductV2`
- `getProductByVersion`
- `placePurchaseOrderForFreeProduct`
- `placePurchaseOrderWithLineCoin`
- `getPurchasedProducts`
- `getReceivedPresents`
- `getSentPresents`
- `notifyProductEvent`
- `getProductValidationScheme`
- `validateProduct`
- `getRecommendationList`
- `getResourceFile`
- `getAutoSuggestionShowcase`
- `getSubscriptionStatus`
- `getSuggestResourcesV2`
- `reserveCoinPurchase`
- `getCoinUseAndRefundHistory`
- `getCoinProducts`
- `getCoinPurchaseHistory`
- `createCombinationSticker`
- `shouldShowWelcomeStickerBanner`
- `stopBundleSubscription`
- `getProductLatestVersionForUser`
- `getMusicSubscriptionStatus`
- `addThemeToSubscriptionSlot`

## SquareBotService

Source: [`protocol/vendor/deps/linejs/base/service/squarebot/mod.ts`](../protocol/vendor/deps/linejs/base/service/squarebot/mod.ts).

Declared request path: `/BP1`.

- `getSquareBot`

## SquareLiveTalkService

Source: [`protocol/vendor/deps/linejs/base/service/livetalk/mod.ts`](../protocol/vendor/deps/linejs/base/service/livetalk/mod.ts).

Declared request path: `/SQLV1`.

- `acceptSpeakers`
- `acceptToChangeRole`
- `acceptToListen`
- `acceptToSpeak`
- `cancelToSpeak`
- `fetchLiveTalkEvents`
- `findLiveTalkByInvitationTicket`
- `forceEndLiveTalk`
- `getLiveTalkInfoForNonMember`
- `getLiveTalkInvitationUrl`
- `getLiveTalkSpeakersForNonMember`
- `getSquareInfoByChatMid`
- `inviteToChangeRole`
- `inviteToListen`
- `inviteToLiveTalk`
- `inviteToSpeak`
- `joinLiveTalk`
- `kickOutLiveTalkParticipants`
- `rejectSpeakers`
- `rejectToSpeak`
- `reportLiveTalk`
- `reportLiveTalkSpeaker`
- `requestToListen`
- `requestToSpeak`
- `updateLiveTalkAttrs`
- `acquireLiveTalk`

## SquareService

Source: [`protocol/vendor/deps/linejs/base/service/square/mod.ts`](../protocol/vendor/deps/linejs/base/service/square/mod.ts).

Declared request path: `/SQ1`.

- `inviteIntoSquareChat`
- `inviteToSquare`
- `getJoinedSquares`
- `markAsRead`
- `reactToMessage`
- `findSquareByInvitationTicket`
- `fetchMyEvents`
- `fetchSquareChatEvents`
- `sendMessage`
- `getSquare`
- `getJoinableSquareChats`
- `createSquare`
- `getSquareChatAnnouncements`
- `leaveSquareChat`
- `getSquareChatMember`
- `searchSquares`
- `updateSquareFeatureSet`
- `joinSquare`
- `getPopularKeywords`
- `reportSquareMessage`
- `updateSquareMemberRelation`
- `leaveSquare`
- `getSquareMemberRelations`
- `removeSubscriptions`
- `getSquareMembers`
- `updateSquareChat`
- `destroyMessage`
- `reportSquareChat`
- `unsendMessage`
- `deleteSquareChatAnnouncement`
- `createSquareChat`
- `deleteSquareChat`
- `getSquareChatMembers`
- `getSquareFeatureSet`
- `updateSquareAuthority`
- `rejectSquareMembers`
- `deleteSquare`
- `reportSquare`
- `getInvitationTicketUrl`
- `updateSquareChatMember`
- `updateSquareMember`
- `updateSquare`
- `getSquareAuthorities`
- `updateSquareMembers`
- `getSquareChatStatus`
- `approveSquareMembers`
- `getSquareStatus`
- `searchSquareMembers`
- `checkJoinCode`
- `createSquareChatAnnouncement`
- `getSquareAuthority`
- `getSquareChat`
- `refreshSubscriptions`
- `getJoinedSquareChats`
- `joinSquareChat`
- `findSquareByEmid`
- `getSquareMemberRelation`
- `getSquareMember`
- `deleteOtherFromSquare`
- `destroyMessages`
- `getCategories`
- `reportSquareMember`
- `getNoteStatus`
- `searchSquareChatMembers`
- `getSquareChatFeatureSet`
- `getSquareEmid`
- `getSquareMembersBySquare`
- `manualRepair`
- `syncSquareMembers`
- `hideSquareMemberContents`
- `markChatsAsRead`
- `reportMessageSummary`
- `getGoogleAdOptions`
- `unhideSquareMemberContents`
- `getSquareChatEmid`
- `getSquareThread`
- `getSquareThreadMid`
- `getUserSettings`
- `markThreadsAsRead`
- `sendSquareThreadMessage`
- `findSquareByInvitationTicketV2`
- `leaveSquareThread`
- `joinSquareThread`
- `updateUserSettings`

## TalkService

Source: [`protocol/vendor/deps/linejs/base/service/talk/mod.ts`](../protocol/vendor/deps/linejs/base/service/talk/mod.ts).

Declared request path: `/S4`.

- `sync`
- `sendMessage`
- `sendCompactMessage`
- `sendCompactPlainMessage`
- `sendCompactE2EEMessage`
- `getProfile`
- `getSettings`
- `sendChatChecked`
- `unsendMessage`
- `deleteOtherFromChat`
- `inviteIntoChat`
- `cancelChatInvitation`
- `deleteSelfFromChat`
- `acceptChatInvitation`
- `reissueChatTicket`
- `findChatByTicket`
- `acceptChatInvitationByTicket`
- `updateChat`
- `getAllContactIds`
- `getBlockedContactIds`
- `getBlockedRecommendationIds`
- `sendPostback`
- `getMessageBoxes`
- `getChatRoomAnnouncementsBulk`
- `getChatRoomAnnouncements`
- `removeChatRoomAnnouncement`
- `createChatRoomAnnouncement`
- `leaveRoom`
- `getRoomsV2`
- `createRoomV2`
- `getCountries`
- `acquireEncryptedAccessToken`
- `blockContact`
- `unblockContact`
- `getConfigurations`
- `fetchOperations`
- `getRepairElements`
- `getSettingsAttributes2`
- `updateSettingsAttributes2`
- `rejectChatInvitation`
- `getE2EEPublicKey`
- `getE2EEPublicKeys`
- `registerE2EEPublicKey`
- `registerE2EEGroupKey`
- `getE2EEGroupSharedKey`
- `getLastE2EEGroupSharedKey`
- `getLastE2EEPublicKeys`
- `negotiateE2EEPublicKey`
- `react`
- `createChat`
- `setChatHiddenStatus`
- `getFollowers`
- `getFollowings`
- `removeFollower`
- `follow`
- `unfollow`
- `bulkFollow`
- `decryptFollowEMid`
- `getMessageReadRange`
- `getChatRoomBGMs`
- `updateChatRoomBGM`
- `blockRecommendation`
- `unblockRecommendation`
- `getRecommendationIds`
- `getExtendedProfile`
- `updateExtendedProfileAttribute`
- `setNotificationsEnabled`
- `syncContacts`
- `findContactsByPhone`
- `findContactByUserid`
- `updateContactSetting`
- `findContactByUserTicket`
- `verifyQrcode`
- `reportAbuseEx`
- `updateProfileAttributes`
- `updateNotificationToken`
- `tryFriendRequest`
- `generateUserTicket`
- `getRecentFriendRequests`
- `resendPinCode`
- `notifyRegistrationComplete`
- `getInstantNews`
- `changeVerificationMethod`
- `getChatEffectMetaList`
- `notifyInstalled`
- `reportDeviceState`
- `sendChatRemoved`
- `startUpdateVerification`
- `inviteIntoRoom`
- `removeFriendRequest`
- `reportProfile`
- `wakeUpLongPolling`
- `updateAndGetNearby`
- `reportSettings`
- `verifyPhoneNumber`
- `isUseridAvailable`
- `registerUserid`
- `finishUpdateVerification`
- `clearRingtone`
- `notifyUpdated`
- `reportPushRecvReports`
- `getFriendRequests`
- `addToFollowBlacklist`
- `removeFromFollowBlacklist`
- `getFollowBlacklist`
- `determineMediaMessageFlow`
- `createSession`
- `cancelReaction`
- `getNotificationSettings`
- `getChats`
- `getChat`
- `getAllChatMids`
- `getPreviousMessagesV2WithRequest`
- `getRecentMessagesV2`
- `getServerTime`
- `getContact`
- `getContacts`
- `getContactsV2`
- `noop`

## Timeline

Source: [`protocol/vendor/deps/linejs/base/timeline/mod.ts`](../protocol/vendor/deps/linejs/base/timeline/mod.ts).

- `initTimeline`
- `createPost`
- `deletePost`
- `getPost`
- `listPost`
- `updatePost`
- `likePost`
- `createComment`
- `sharePost`
- `uploadNoteMedia`
- `uploadNoteCommentImage`

## LineObs

Source: [`protocol/vendor/deps/linejs/base/obs/mod.ts`](../protocol/vendor/deps/linejs/base/obs/mod.ts).

- `getMessageDataUrl`
- `getMessageMetadataUrl`
- `downloadMessageData`
- `getMessageObsMetadata`
- `uploadObjTalk`
- `uploadObjectForService`
- `downloadObjectForService`
- `uploadMediaByE2EE`
- `downloadMediaByE2EE`

## Scope of this inventory

The library list covers the exported LINE service wrappers and the Timeline/OBS helpers. It excludes general runtime utilities, login orchestration, transport internals, generated Thrift structures, and high-level convenience methods that wrap the same services. A declaration is evidence of local wrapper code, not proof that the server currently accepts the operation. Consult the linked source for signatures and comments.
