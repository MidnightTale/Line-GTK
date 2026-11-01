*Reverse engineering · REA · LINE for Mac 26.1.0 (3865)*

# Kiến trúc LINE for Mac, bóc tách từ binary

> Nguồn: artifact https://claude.ai/artifact/PQLwVsu1ov2wa5mCMaJm6J (tác giả bên ngoài, chưa kiểm chứng độc lập). Lưu ngày 2026-10-28 làm tài liệu tham khảo; các hình SVG gốc được thay bằng chú thích văn bản. Đây là suy luận từ binary của ứng dụng chính thức, không phải đặc tả giao thức.

Tám hình và năm mục tham chiếu, từ topology tiến trình, đăng nhập, khoá E2EE đến API, tính năng ẩn và code đã xác minh. Nguồn: Mach-O, chữ ký, entitlements, Info.plist, bảng symbol và string của bản Mac App Store
(team VUTU7AKEUR, arm64 sha256 5184dc59…). Chưa decompile và chưa chạy app: mọi luồng điều khiển là suy luận từ symbol và log string, được ghi rõ ở từng chú thích.

*01 · Deployment*

## Tiến trình, sandbox và kênh IPC
Quan sát: entitlements, Info.plist, otool. LINE spawn LineCall qua `CallPluginController` và nói chuyện bằng TCP 127.0.0.1 (`LcIPCServer`, khoá lưu Keychain). Năm XPC Audio/Media/SeekPreview/VideoPreview×2 gần như cùng một binary (khác ~2 KB) dùng FFmpeg 3.x; `FFmpegService` chạy dòng lệnh transcode. Suy luận: tiến trình MCP stdio con thừa hưởng sandbox của LINE.

> **Hình:** Deployment view of the LINE main process, the LineCall helper, the share extension, six FFmpeg XPC services, locally spawned MCP stdio servers and the LY Corp backends they reach.

*02 · Layer stack*

## Các tầng code trong binary chính (102 MB)
Dựng từ 9.375 symbol export, 52 class ObjC và các log tag trong string (vd. `[ChatService]` 89 dòng, `[E2EE]` 89, `[DeeplingoKit]` 152). Ranh giới tầng là suy luận theo tên; chưa có call graph vì thiếu Ghidra/Hopper.

> **Hình:** Layer stack of the LINE main executable from the macOS platform runtime up through storage and transport, the messaging core, the AI layer, feature services and the Qt presentation layer.

*03 · Dependency graph*

## Đồ thị liên kết framework
Quan sát trực tiếp từ `LC_LOAD_DYLIB` (lát arm64). LINE và LineCall link cùng 7 framework nội bộ/bên thứ ba, nên gộp làm một node. Không có vòng phụ thuộc. Các XPC media link riêng 7 dylib FFmpeg 3.x (không vẽ ở đây).

> **Hình:** Dependency graph showing both LINE executables linking YukiDesktop, AMPUtilVideo, skottie, Sentry, NeuralLine, AMPKit and Qt, with NeuralLine, AMPKit and Qt each reached by two parents.

*04 · Architecture*

## AI Agent và MCP client
Quan sát: namespace `MCP::` (SSE, Streamable HTTP, `ProcessStdioChannel`), `deeplingo::OpenAIClient`/`NativeMacClient`, cầu nối `webview_*`/`load_mcp_config`/`enable_mcp_server_tool`. Endpoint remote là template dev với token placeholder; log `mcpServers disabled` gợi ý mặc định tắt. Toàn bộ Agent nằm sau cờ prototype `proto_ai_agent` (LYXPE-944, “AI Agent PoC”), xem mục 12. Chưa xác định vị trí file config.

> **Hình:** Architecture of the LINE AI agent: the React agent panel bridges into AIAgent, which drives DeeplingoKit toward cloud LLMs, Apple on-device speech, built-in LINE tools and an MCP manager with SSE, HTTP and stdio transports.

*05 · Sequence*

## Luồng nhận một tin nhắn chat
Tái dựng từ log string (`[POLLING]`, `[OP_FETCHER]`, `[E2EE] ADD DECRYPT PENDING QUEUE`, `failed to find creator's pub key`). Thứ tự là suy luận hợp lý, chưa quan sát runtime. Talk API dùng Thrift Compact trên LEGY (`/S4`, `/api/v4/TalkService.do`).

> **Hình:** Sequence reconstructed from log strings: the transport long-polls LEGY for operations, OperationFetcher merges them, Letter Sealing decrypts with a cached or freshly fetched key, and the message is written to msg.db and rendered.

*06 · Architecture*

## Ngăn xếp cuộc gọi thoại/video
Quan sát: log `[Call][Ampkit] OnCallState`, class `NeuralLine::UltraFace/LNet/FSANet/BSMobileNet/PPHumanSeg`, Yuki chứa `kuru`, LuaBridge, Effekseer, OpenSSL. Không có file model trong bundle nên đường tải model qua `PluginDownloader` là suy luận.

> **Hình:** Architecture of LINE calls: the main app spawns the LineCall helper over a loopback IPC channel, LineCall drives AMPKit for WebRTC media to LY servers and YukiDesktop effects backed by NeuralLine MNN models.

*07 · Sequence*

## Đăng nhập bằng mã QR
Quan sát: tên hàm Thrift `createSession`, `createQrCode`, `checkQrCodeVerified`, `createPinCode`, `checkPinCodeVerified`, `verifyCertificate`, `qrCodeLoginForSecure`. Thứ tự là suy luận theo tên hàm. Ngay sau bước này là trao đổi khoá E2EE ở hình 08.

> **Hình:** Sequence of the LINE for Mac QR login: the Mac opens a session and shows a QR code, the phone scans it, a PIN is confirmed on new devices or the certificate is reused on trusted ones, and the server returns access tokens.

*08 · Architecture*

## Cây khoá Letter Sealing
Quan sát: `X25519`, `registerE2EEPublicKey`, `requestE2EEKeyExchange` (PIN, verifier), `My Private Key Saved : keyId`, `RegisterE2EEGroupKey`, `myEncryptedSharedKey`, `CommLib::E2EEUtil::encryptByAes256Ctr`, `extractHKDF`, `E2EEChunkedHMACValidator`, field `keyMaterial`/`salt`/`nonce`. Chi tiết KDF và AES-GCM cho tin văn bản lấy từ tài liệu công khai của LINE. Việc Mac nhận khoá từ điện thoại thay vì tự sinh là suy luận.

> **Hình:** Key hierarchy of LINE Letter Sealing: the device X25519 key pair arrives from the phone over a PIN exchange and is kept in Keychain, then derives one-to-one secrets, unwraps per-group keys, and those message keys wrap the random key material that encrypts each file.

*09 · Lifecycle*

## Từ lúc cài đến khi nhận thông báo

- **Cài:** bản Mac App Store, sandbox, không có entitlement `aps-environment`, nên không nhận push của Apple.
- **Mở app:** chặn chạy hai bản, kiểm tra `update_info.json`, bật Sentry, thử `autoLogin` (cần authToken, userId, everLoginedFlag và cài đặt bật).
- **Đăng nhập:** QR (hình 07) hoặc email/mật khẩu (`loginV2`/`loginZ`). Token làm mới qua `/EXT/auth/tokenrefresh/v1`.
- **Nhận khoá E2EE** từ điện thoại qua cửa sổ PIN, lưu vào Keychain (hình 08).
- **Đồng bộ:** conninfo → `FULLSYNC` (hồ sơ, bạn bè, nhóm) → `OperationFetcher` kéo theo revision → `msg.db` + FTS.
- **Gửi:** `SendMessageTask` → mã hoá → file lên OBS (`UploadContentTask`) → `sendMessage`; lỗi thì vào hàng `AUTORESEND`.
- **Nhận và thông báo:** long-poll LEGY khi app đang chạy → giải mã (hình 05) → toast qua `UNUserNotificationCenter`. Tắt app thì không có thông báo.
- **Đọc:** `sendChatChecked`, `ChatReadCounter` tính số người đã đọc.
- **Đăng xuất:** `logoutZ`. Chưa rõ `msg.db` và khoá trong Keychain có bị xoá.

*10 · Letter Sealing*

## Vì sao tin hiện “couldn’t be decrypted”
Tin đến trước khoá: LINE đưa tin vào hàng chờ (`ADD DECRYPT PENDING QUEUE`), xin khoá qua `GetE2EEPublicKey`/`GetE2EEGroupKey` rồi giải mã lại. Nếu lỗi tự hết sau vài giây thì đó là trường hợp này. Lỗi đứng yên khi hết số lần thử (`max retry count`), khi Mac mất khoá riêng (`INVALID_MY_E2EE_KEY_DETECTED : EMPTY KEYCHAIN`), khi nhóm không giải được khoá chung (`failed to find creator’s pub key`) hoặc khi spec mới hơn (`INVALID SPEC VERSION`). Binary còn chứa lệnh debug `E2EE KEY DUMP` in khoá ra; chưa biết bản phát hành có mở được không.

*11 · REST*

## Các endpoint REST trong binary
Host do server cấp lúc chạy qua `ConnInfoManager`; tiền tố `/ext/note/nt`, `/ext/album/api` gợi ý LEGY làm gateway. Header: `X-Line-Access`, `X-Line-Application`, `X-Line-ChannelToken`, `X-LAL`. Tin nhắn, bạn bè, nhóm, OpenChat và E2EE không đi REST mà qua Thrift (`/S4`, `/SQ1`). Đây là API riêng: dùng ngoài app vi phạm điều khoản.

| Dịch vụ | Endpoint | Code  |
| Keep | /api/v27/keep/{init,sync,fetch,get,create,update,delete,size}.json, contents/{pin,unpin}, message/delete, obs/delete | KP::command::Keep*  |
| Note / group posts | /api/v57/post/{create,get,list,update,delete,getStatus,share,sendPostToTalk,report}.json, comment/*, like/*, announce/*, hashtag/*, search/note, grouphome/isnew, otoaccount/* | note::command::*  |
| Home / Timeline | /api/v1/home/{profile,cover,groupprofile}, /api/v24/group/profileimage/list.json, /api/v1/userpopup/getDetail.json, /api/v2/pageinfo/get.json | TL::command::*  |
| Notifications | /api/v1/notification/fetch/new, /api/v6/notification/fetch/push, /PUSH/1/subs | TL::command::Fetch*  |
| Album | /v6/albums, /v2/albums, /v2/photos, …/photos/{create,delete,obsDownloadInfo} | AlbumService  |
| Media (OBS) | /oa/r/{svc}/{sid}/{oid}/{object_info,playback,copy}.obs | obs-*.line-apps.com  |
| Login / token | /acct/lgn/sq/v1, /acct/lgn/secpwless/v1, /EXT/auth/tokenrefresh/v1, hub.line.me/api/auth/dtlogin | AUTH · SessionManager  |
| Premium | /EXT/line-premium/common/thrift/{status,line-premium-status}, /v5/user/premium/latestTerms | Thrift over REST  |
| Terms / notices | /v1/{app}/{lang}/terms/{doc}, …/document/notice, …/notification | LanService  |
| Group call | /EXT/groupcall/youtube-api/api | YouTubeService  |
| App update | desktop.line-scdn.net/osx/v1/real/update_info.json, …/win/plugin/v1/plugin_remote_info.json | UpdateInfoChecker  |
| AI | /v1/chat/completions, /v1/audio/{speech,transcriptions} → line-x-openai.line-apps.com | deeplingo::OpenAIClient  |

*12 · Prototype flags*

## Tính năng AI ẩn sau cờ prototype
Các tính năng AI không xuất hiện trong menu thường. Mỗi tính năng là một cờ `proto_*` gắn mã ticket nội bộ (`LFO-`, `LYXPE-`); mục “[Beta] GPT-4o Voice Chat” nằm trong menu debug cạnh “Debug Window” và “KeepTest”. Cột mặc định đọc từ chuỗi tuỳ chọn kiểu `on,*off` nằm gần cờ (dấu * là mặc định); “(?)” vì chưa chắc chuỗi đó thuộc cờ nào, “—” là không thấy. Chưa rõ điều kiện mở panel prototype; có lệnh `/office` kiểm tra máy có nằm trong mạng văn phòng nội bộ.

| Cờ | Ticket | Tính năng | Mặc định  |
| proto_chat_openai_chatGPT | LFO-4444 | OpenAI ChatGPT support | off (?)  |
| proto_chat_openai_GPT4o_voice_chat | LYXPE-189 | GPT-4o Voice Chat | —  |
| proto_chat_ai_summary | LYXPE-389 | Chat Message Summary with ChatGPT | on (?)  |
| proto_chat_ai_search | LYXPE-530 | Chat Message Search with ChatGPT | —  |
| proto_chat_translated_text | LFO-4589 | Chat Auto-Translated Text Message | —  |
| proto_ai_agent | LYXPE-944 | AI Agent PoC (WebView) · MCP | —  |
| proto_ocr_translate | LYXPE-1011 | OCR + translate | —  |
| proto_sticker_nlp_search | LYXPE-1025 | Sticker search by natural language | —  |
| proto_chat_image_search_ocr | LFO-4860 | Chat Image Search with OCR | —  |
| proto_call_video_bg | LFO-4326 | Call video background | —  |
| proto_remove_bg | LFO-4141 | Viewer: remove background | —  |
| proto_quick_screenshare | LFO-4462 | Quick screen share | —  |

Lệnh gõ trong khung chat tìm thấy trong binary: `/dice`, `/calc`, `/flash`, `/tts`, `/version`, `/sticker`, `/chatgpt`, `/chatgpt-image`, `/assistant-create`, `/assistant-info`, `/assistant-clear`, `/assistant-delete`, `/proto`, `/prototype`, `/filelog`, `/accessibility`, `/office`, `/tokenver`, `/sendlog`, `/leaverooms`.

*13 · Phụ lục*

## Code đã xác minh bằng disassembly
Khác với các mục trên (suy luận từ tên hàm và log), phần này đọc thẳng mã máy arm64 nên là **quan sát trực tiếp**. Nguồn: `otool -tV` trên `NeuralLine.framework`.

**`NeuralLine::AdaptiveFrame<float>::updateDiff()`** tính trung bình độ lệch tuyệt đối từng điểm ảnh so với frame trước (vector hoá bằng lệnh NEON `fabd.4s`), rồi lưu hai số: độ lệch trung bình và độ sáng trung bình.

**`NeuralLine::AdaptiveFrame<float>::shouldSkip()`** quyết định có bỏ qua một frame hay không:

- Độ lệch ≥ ngưỡng (có chuyển động) → trả về `false`, tức **chạy model**.
- Độ lệch < ngưỡng (gần như đứng yên) → tăng một bộ đếm và chỉ bỏ qua **xen kẽ một frame**, không bao giờ bỏ hai frame liền.

Ý nghĩa: cảnh tĩnh thì tiết kiệm GPU cho tách nền và dò mặt, nhưng vẫn chừa một nhịp chạy định kỳ để kết quả không bị trôi. Đây thay cho dòng “chưa kiểm chứng thuật toán `shouldSkip`” ở các bản trước.

**`encryptE2EEMessageInternal`** (Ghidra `FUN_101ca1810`, decompile từ binary chính). Hàm này kiểm tra và điều phối, chưa phải nơi mã hoá:

- Chỉ chấp nhận **spec version 1 hoặc 2**; giá trị khác bị từ chối và log `INVALID SPEC VERSION`. Xác nhận Letter Sealing đúng hai phiên bản.
- Phân nhánh theo spec rồi gọi hàm mã hoá thật (`FUN_101ca2fd8`), truyền kèm spec version.
- Hai biến thể log ENCRYPT E2EE cho chat 1-1 và chat nhóm, khác nhau ở thứ tự tham số.

Còn chờ Ghidra để xác minh tiếp: chi tiết KDF/AES trong `FUN_101ca2fd8`, `OperationFetcher::startFetch` (độ trễ long-poll, số lần thử) và `ConnInfoManager` (cách chọn host). Mỗi hàm là một lượt phân tích lại ~24 phút vì không tái dùng được snapshot trên máy này.

