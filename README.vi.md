# Line-GTK

Ứng dụng LINE không chính thức dành cho Linux, với giao diện GTK4/Libadwaita và tiến trình giao thức Deno sử dụng [linejs](https://github.com/evex-dev/linejs).

**README:** [English](README.md) · Tiếng Việt · [日本語](README.ja.md)

> Nhánh `personal/features` dành cho phát triển tính năng cá nhân, tách khỏi `main` dùng để gửi pull request lên dự án gốc. Nhánh này tích hợp giao diện và phông chữ tiếng Nhật, giải mã tin nhắn, thông tin nhóm và đăng nhập bằng email.

![Cửa sổ chính Line-GTK](assets/761546283_1013837091419784_4898538377611644175_n.jpg)

## Tính năng

- [x] Đăng nhập QR như thiết bị bổ sung; đăng nhập email và mật khẩu, xác minh trên điện thoại khi cần
- [x] Trò chuyện, tin nhắn, sticker, bạn bè, ảnh, video, tin nhắn thoại và tệp
- [x] Trả lời trích dẫn, thu hồi tin nhắn, biểu cảm, emoji Unicode và hiển thị emoji LINE nhận được
- [x] Lọc và tìm kiếm cuộc trò chuyện; ẩn, ghim và chặn liên hệ
- [x] Thông báo sự kiện nhóm và tên người gửi
- [x] Album, tải ảnh gốc và trình xem ảnh, video, PDF, văn bản trong ứng dụng
- [x] Dán và kéo thả tệp đính kèm
- [x] Xem trước sticker trong thông báo, đồng bộ chưa đọc, khôi phục sau chế độ ngủ hoặc mất mạng
- [x] Tải xuống, khay hệ thống, giao diện, Discord RPC; giao diện tiếng Việt (mặc định), Thái, Anh và Nhật
- [ ] Gọi thoại (đang phát triển)
- [ ] Chuyển đổi tài khoản

### Thông báo đã đọc và trạng thái chưa đọc

Bật **Không đánh dấu đã đọc** ở đầu cửa sổ để ứng dụng ngừng gửi thông báo đã đọc. Thiết lập được giữ sau khi khởi động lại và huy hiệu chưa đọc vẫn hiển thị. Tắt thiết lập này không đánh dấu tất cả là đã đọc ngay; thao tác thông thường như mở cuộc trò chuyện sẽ tiếp tục gửi thông báo. Với cuộc trò chuyện đang mở ở phần tin mới nhất, nhấp vào nội dung hoặc ô nhập, gõ phím hay cuộn có thể gửi thông báo đã đọc. Việc đọc lịch sử cũ, bật tạm dừng toàn cục hoặc tạm dừng theo liên hệ sẽ ngăn thao tác này. Thiết lập không ảnh hưởng đến trạng thái đã đọc trước đó hoặc từ thiết bị khác.

Để tạm dừng cho một người, nhấp chuột phải vào cuộc trò chuyện và chọn **Không đánh dấu đã đọc cho người bạn này**. Ảnh đại diện sẽ có biểu tượng che mắt. Mở cuộc trò chuyện hoặc nhận tin mới vẫn giữ trạng thái chưa đọc. Dùng cùng menu để bật lại; riêng thao tác bật lại không gửi thông báo đã đọc. Thiết lập được lưu theo tài khoản. Việc bật lại cho một người không có tác dụng khi tạm dừng toàn cục đang bật hoặc tự động đánh dấu đã đọc đang tắt.

Dùng **Đánh dấu đã đọc đến đây** ở đầu cuộc trò chuyện hoặc trong menu chuột phải để chỉ đánh dấu những tin đã nhận đến thời điểm đó. Thiết lập tạm dừng vẫn giữ nguyên nên tin mới tiếp tục chưa đọc. Chỉ phạm vi gửi thành công mới được đồng bộ; tin đến trong lúc gửi hoặc yêu cầu thất bại không xóa nhầm số chưa đọc. Thông báo đã đọc từ thiết bị khác vẫn cập nhật huy hiệu, cuộc trò chuyện đang mở và khay hệ thống. Đồng bộ này không gửi thêm thông báo đã đọc và kiểm tra số chưa đọc trên máy chủ khi cần để tránh xóa nhầm tin mới.

### Tin nhắn và quản lý cuộc trò chuyện

Nhấn mũi tên cạnh tin nhắn hoặc chọn **Trả lời** trong menu chuột phải để trích dẫn. Hủy bằng × hoặc Esc; chuyển sang cuộc trò chuyện khác cũng xóa trích dẫn. Nút mặt cười hoặc mục **Biểu cảm** cho phép chọn sáu biểu cảm chuẩn, emoji LINE dùng gần đây hoặc emoji thuộc bộ đã sở hữu. Nhấn biểu cảm dưới bong bóng để thêm hoặc gỡ; nhấp chuột phải hoặc nhấn giữ để xem ai đã dùng. Biểu cảm từ người khác và thiết bị khác được lưu theo tài khoản. Sáu biểu cảm chuẩn dùng biểu tượng LINE trong `assets/reactions`. LINE có thể từ chối gửi biểu cảm emoji tùy điều kiện như quyền sở hữu; ứng dụng PC chính thức cũng chỉ hiển thị biểu cảm emoji LINE đã nhận.

Nhấp chuột phải vào cuộc trò chuyện riêng và xác nhận **Chặn** để chặn người đó trên LINE. Bỏ chặn ở cùng menu hoặc **Cài đặt → Tính năng → Người dùng bị chặn**. Lịch sử được giữ lại, danh sách có dấu chặn, và ô nhập cùng yêu cầu gửi trong cuộc trò chuyện riêng bị vô hiệu hóa. Tin nhắn của người đó trong nhóm vẫn hiển thị. Danh sách chặn trên máy chủ được làm mới khi khởi động, sau khi máy thức dậy và khi thiết bị khác thay đổi; nếu yêu cầu lỗi, trạng thái đã xác nhận trước đó được giữ. Chặn không thay đổi thiết lập đã đọc. Cách xử lý trong nhóm theo [trợ giúp LINE](https://help2.line.me/line/smartphone?contentId=200002013&lang=ja).

Nhấp chuột phải để ghim hoặc bỏ ghim cuộc trò chuyện với bạn bè. Mục ghim nằm trên cùng; trong mỗi nhóm, cuộc trò chuyện vẫn sắp theo thời gian mới nhất. Bộ lọc tìm kiếm, danh mục và ẩn vẫn áp dụng. Ghim được lưu theo tài khoản trên máy này và không gửi thông báo đã đọc. Nhấp chuột phải để ẩn cuộc trò chuyện khỏi danh sách, kết quả tìm kiếm và mục gần đây trong khay. Khôi phục tại **Cài đặt → Tính năng → Cuộc trò chuyện đã ẩn**. Lịch sử không bị xóa; ẩn hoặc khôi phục không đánh dấu đã đọc. Nếu ẩn cuộc trò chuyện đang mở, lựa chọn được bỏ mà không mở cuộc khác. Trạng thái ẩn vẫn còn sau tin mới và khởi động lại, nhưng không đồng bộ sang thiết bị khác; thiết lập thông báo độc lập.

Chọn **Thu hồi** trên tin đã gửi và xác nhận để yêu cầu LINE thu hồi. Nội dung chỉ được thay bằng thông báo thu hồi sau khi LINE xác nhận thành công; nếu thất bại, tin gốc được giữ. LINE quyết định giới hạn thời gian và điều kiện, và hiển thị thông báo chuẩn cho người nhận. Sự kiện thu hồi từ thiết bị khác cũng được áp dụng và lưu ID theo tài khoản để tải lại lịch sử hoặc khởi động lại không khôi phục nội dung. Ảnh và tệp đã tải về máy không bị xóa.

Bộ chọn sticker hiển thị bản xem trước 160 px khi rê chuột, có thể phát ảnh động đã lưu trong bộ nhớ đệm. Nút emoji trong ô nhập chèn emoji Unicode tại vị trí con trỏ dưới dạng văn bản, dùng phông emoji hệ thống (`noto-fonts-emoji` trên Arch). Emoji LINE nhận được hiển thị dưới dạng ảnh tĩnh trong nội dung; tối đa bốn ảnh được tải nền đồng thời vào bộ nhớ đệm. Nếu ảnh không tải được, văn bản thay thế vẫn hiện; phần trích dẫn và bản xem trước cũng dùng văn bản thay thế. Chưa hỗ trợ gửi bộ emoji LINE đã mua hoặc phát emoji động.

Trong bộ chọn sticker, dùng **Từ LINE STORE** để xem trước và tải bộ của nhà sáng tạo qua liên kết hoặc ID. **Nhập tệp PNG** cho phép xem trước, chọn ảnh và lưu vào bộ mới hoặc bộ cá nhân hiện có (tối đa 100 tệp mỗi lần, 10 MiB và 2048 × 2048 pixel mỗi ảnh; PNG và APNG). Ứng dụng lưu bản sao riêng theo tài khoản, nên di chuyển tệp gốc hoặc xóa bộ nhớ đệm không làm mất bộ ảnh. Có thể đổi tên, thêm ảnh, chỉnh kích thước/nền và xóa ảnh hoặc bộ. Bộ LINE đã sở hữu gửi sticker gốc; ảnh nhập gửi dưới dạng ảnh thông thường. Ảnh cá nhân mặc định giữ nguyên kích thước và nền.

Ảnh APNG động được giữ nguyên bản gốc và chuyển sang GIF khi gửi. Chọn **Tối ưu ảnh** để xem dung lượng trước/sau, tùy chọn giảm kích thước, tốc độ khung hình và số màu GIF. Ảnh tĩnh dùng nén PNG không mất dữ liệu; thiết lập được lưu cho từng ảnh. GIF có thể lớn hơn bản APNG và không giữ được mức trong suốt trung gian.

Sticker động chưa lưu thiết lập mặc định dùng **128 px, 10 fps, 64 màu** để giảm dung lượng bên nhận. Với ảnh đã có thiết lập cũ, chọn **GIF nhẹ** để xem trước cấu hình này rồi lưu hoặc gửi. Có thể giảm tiếp xuống 96 px, 5–8 fps hoặc 32 màu; ảnh tĩnh vẫn giữ kích thước gốc theo mặc định.

### Nhóm, thông báo và khôi phục kết nối

Các sự kiện mời, tham gia, rời và xóa thành viên nhóm hiển thị thành dòng thông báo có tên, kể cả khi liên quan nhiều người. Tên được tải nền. Sự kiện máy này nhận được lưu theo tài khoản và hiển thị cùng tin nhắn theo thời gian sau khi mở lại hoặc khởi động lại. Giới hạn là 500 sự kiện mỗi cuộc trò chuyện và 5.000 mỗi tài khoản; xóa bộ nhớ đệm tin nhắn cũng xóa chúng. Không thể trả lời dòng thông báo và ID sự kiện không được dùng để gửi trạng thái đã đọc. API lịch sử không cung cấp sự kiện nhóm cũ; sự kiện bị lỡ khi ứng dụng đóng hoặc ngắt kết nối chỉ xuất hiện nếu LINE gửi lại. Bong bóng nhóm và trò chuyện nhiều người hiển thị tên người gửi, kể cả người chưa kết bạn, và cập nhật tên cùng tên trong trích dẫn sau khi tra cứu nền.

Thông báo nhóm hiển thị **Người gửi (Tên nhóm)** cho văn bản, ảnh và sticker. Việc tra tên đợi tối đa 1,2 giây; nếu cần, ứng dụng hiển thị tên tạm để phân biệt thành viên rồi âm thầm cập nhật cùng thông báo. Thông báo sticker hiển thị ảnh tĩnh nếu máy chủ thông báo hỗ trợ ảnh trong nội dung. Ảnh cũng được đợi tối đa 1,2 giây; có thể hiện thông báo chữ trước rồi cập nhật âm thầm. Thông báo đã đóng không bật lại khi ảnh đến muộn. Máy chủ khác dùng gợi ý ảnh và biểu tượng sticker, với chữ dự phòng. Ảnh thông báo là PNG trong suốt tối đa 200 × 100 px và bị xóa cùng bộ nhớ đệm sticker. Nhấp thông báo mở cuộc trò chuyện; thiết lập tắt tiếng vẫn áp dụng.

Sau khi máy thức dậy, ứng dụng đợi hai giây rồi kết nối lại luồng nhận bằng phiên đã lưu, làm mới danh sách và 80 tin gần nhất của cuộc trò chuyện đang mở, rồi hợp nhất không trùng lặp. Bản nháp, trích dẫn, vị trí cuộn, mục ẩn/ghim và thiết lập tạm dừng đã đọc được giữ. Chỉ làm mới không gửi trạng thái đã đọc. Tin đang gửi được hợp nhất sau khi gửi xong. Nếu chưa kết nối được, giao diện hiện có vẫn giữ và ứng dụng thử lại sau 3, 10 và 30 giây. Sau đó có nút **Thử lại**; mạng trở lại cũng kích hoạt khôi phục. Ứng dụng dùng thông báo thức dậy của logind, với kiểm tra khoảng cách thời gian định kỳ làm phương án dự phòng. Bản xem trước trò chuyện được cập nhật khi tải tin; tải nền tối đa hai cuộc cùng lúc và trở về **Nhấn để mở** nếu không lấy được tin mới nhất.

### Media và album

Nhấn nút phát trên ảnh thu nhỏ video để mở trình xem có điều khiển phát/tạm dừng, vị trí, thời gian và âm lượng. Có thể phát lại sau khi kết thúc, lưu video gốc hoặc đóng để dừng. Cần backend media GTK và codec phù hợp. Nếu không phát được, dùng **Thử lại**, **Mở bằng ứng dụng khác** hoặc tải xuống. Nếu bản gốc trên LINE đã hết hạn, ứng dụng báo lỗi thay vì lưu ảnh thu nhỏ như video. Video trong album và bản xem trước sticker động chưa được hỗ trợ.

Dùng **Album** ở đầu cuộc trò chuyện để mở album của nó, hoặc **Tất cả cuộc trò chuyện** để xem toàn bộ. Khi chưa chọn cuộc trò chuyện, danh sách tổng hiện mặc định. Mở ảnh để phóng to và lưu bản gốc từ ảnh hoặc trình xem. Vị trí lưu và hộp thoại theo thiết lập tải ảnh hiện có; lưu thông thường không ghi đè tệp trùng tên. Ảnh gốc được tải từng đợt 12 ảnh, tối đa ba lượt đồng thời, và dùng lại từ bộ nhớ đệm. Hỗ trợ JPEG, PNG, GIF, WebP; tối đa 50 MB mỗi ảnh. Video trong album được đánh dấu chưa hỗ trợ; chưa thể tạo album hoặc thêm/xóa ảnh. Dùng **Thử lại** khi tải lỗi hoặc làm mới thông tin album đã cũ.

## Yêu cầu

Đăng nhập email xác thực ứng dụng như thiết bị PC bổ sung và có thể cần nhập mã xác minh trên điện thoại. Thông thường ứng dụng khôi phục phiên bằng thông tin xác thực PC. Bật **Lưu thông tin đăng nhập và tự động đăng nhập khi khởi động** để lưu email và mật khẩu vào kho khóa hệ điều hành sau khi đăng nhập thành công. Có thể thêm, đổi hoặc xóa tại **Cài đặt → Tài khoản → Tự động đăng nhập khi khởi động**.

Tự động đăng nhập chỉ chạy một lần khi khởi động. Ứng dụng ưu tiên phiên đã lưu và chỉ thử email khi phiên thiếu hoặc hết hiệu lực. Lỗi mạng tạm thời không kích hoạt đăng nhập bằng mật khẩu; bị ngắt sau khi đăng nhập từ PC khác cũng không tự đăng nhập lại. Mã xác minh trên điện thoại sẽ được hiển thị khi cần; sai thông tin đăng nhập đưa về màn hình đăng nhập thủ công. Email và mật khẩu được lưu trong kho khóa tương thích Secret Service như GNOME Keyring, không nằm trong tệp cấu hình hoặc tham số dòng lệnh. Nếu lưu hoặc đọc kho khóa thất bại, tự động đăng nhập bị tắt hoặc chuyển sang thủ công. Có thể cần mở khóa kho khóa. Xóa thông tin hoặc đăng xuất thủ công sẽ tắt tự động đăng nhập và xóa thông tin đã lưu. Tính năng gọi thoại thử nghiệm yêu cầu đăng nhập QR.

| Phụ thuộc | Ghi chú |
| --- | --- |
| Rust | Bản stable |
| GTK4 + Libadwaita | GTK4 4.14+ và Libadwaita 1.5+; trên Arch: `pacman -S gtk4 libadwaita` |
| Deno | Deno 2; biên dịch giao thức khi build; chạy từ mã nguồn dùng `deno` hoặc `DENO` |
| Python 3 | Kiểm tra phụ thuộc và gói |
| ffmpeg / ffprobe | Ghi âm và ảnh thu nhỏ video |
| libsecret (`secret-tool`) + Secret Service | Cần cho tự động đăng nhập email |
| Tùy chọn | `pdftoppm`, `pdftotext` để xem trước PDF |

Ứng dụng chạy trên Wayland và X11; chủ yếu được thử trên các desktop dựa trên Arch.

## Cài đặt

### Arch / AUR

```bash
# Bản phát triển
yay -S line-gtk-git

# Bản phát hành gắn thẻ
yay -S line-gtk
```

### Gói Flatpak

```bash
flatpak install --user ./line-gtk-0.1.2-x86_64.flatpak
flatpak run dev.linegtk.LineGtk
```

## Build và chạy

Tiếng Việt và tiếng Anh dùng `Noto Sans` theo mặc định, kể cả phông dự phòng cho giao diện trò chuyện và ô nhập khi chọn phông tùy chỉnh. Trên Arch, cài `noto-fonts`. Thiết lập tiếng Việt cũ dùng `Noto Sans Thai` được chuyển tự động; phông tùy chỉnh và thiết lập dùng phông hệ thống được giữ. Khi chọn tiếng Nhật với phông mặc định, ứng dụng dùng `Noto Sans CJK JP`; phông tùy chỉnh không đổi. Trên Arch, cài `noto-fonts-cjk` cho tiếng Nhật.

```bash
git clone https://github.com/MidnightTale/Line-GTK.git
cd Line-GTK
cargo build --release
./scripts/build-sidecar.sh
LINE_GTK_ROOT="$PWD" ./target/release/line-gtk
```

Quá trình build tự tải thư viện đã ghim phiên bản và checksum, gồm linejs, từ GitHub vào `protocol/vendor/deps` (không lưu mã thư viện trong Git). Manifest và `protocol/deno.lock` vẫn được lưu trong Git; không liên hệ JSR. Lần build đầu cần GitHub, npm registry và runtime biên dịch Deno. Sau khi lưu cache, dùng `DENO_OFFLINE=1 ./scripts/build-sidecar.sh` để chặn tải phụ thuộc. Xem [ghi chú về phụ thuộc và khôi phục kết nối](docs/protocol-reliability.md).

## Kiểm tra khi phát triển

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
deno task --config protocol/deno.json check
```

## Tích hợp bản fork

Nhánh này tích hợp các thay đổi đã ghi từ [shirafukayayoi/Line-GTK](https://github.com/shirafukayayoi/Line-GTK) đến commit `0907007` của `personal/features`, và giữ tiếng Việt là ngôn ngữ mặc định của ứng dụng. Xem [ghi chú tích hợp](docs/fork-shirafukayayoi.md) để biết tính năng và giới hạn.

## Giải mã tin nhắn và thông tin nhóm

Line-GTK dùng linejs 3.5.0 để xử lý nội dung tin nhắn hiện `(none)`. Nếu lần đăng nhập trước lưu sai khóa mã hóa, dùng nút làm mới khóa tại **Cài đặt → Tài khoản** hoặc trên bong bóng báo lỗi giải mã để xác thực lại bằng QR trên điện thoại. Khóa riêng đã lưu được giữ và thế hệ khóa đúng được lấy từ điện thoại. Thông tin nhóm được tải theo từng đợt; tên và ảnh được lưu để hiển thị sau khi khởi động lại.

## Cảnh báo

Đây là ứng dụng không chính thức. Việc sử dụng có thể vi phạm Điều khoản dịch vụ của LINE. Nên dùng tài khoản phụ hoặc phiên thiết bị bổ sung. Thay đổi đăng nhập hay mã hóa của LINE có thể khiến ứng dụng ngừng hoạt động.

## Ghi nhận

Giao thức và mã hóa đầu cuối truy cập LINE qua [linejs](https://github.com/evex-dev/linejs) của [evex-dev](https://github.com/evex-dev). Ứng dụng này là giao diện GTK dùng thư viện đó và không liên kết với LINE Corporation hay evex-dev.

## Giấy phép

[GPL-3.0-or-later](LICENSE)
