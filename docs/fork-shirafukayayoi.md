# Tham khảo fork shirafukayayoi và định hướng tích hợp

Repo đã tích hợp các thay đổi từ [shirafukayayoi/Line-GTK](https://github.com/shirafukayayoi/Line-GTK) đến các mốc nêu bên dưới. Tài liệu này ghi lại kết quả khảo sát ngày **22/10/2026** và cập nhật trạng thái tích hợp vào repo này.

Đã tích hợp các commit của `main` đến `727124f` và `personal/features` đến `0907007` vào repo này. Giữ tiếng Việt mặc định, bổ sung bản dịch cho các khóa mới và giữ các thay đổi cục bộ về kết nối/đóng gói. Các giới hạn của tính năng được ghi bên dưới vẫn áp dụng.

## Kho nguồn và các nhánh

| Nguồn | Vai trò | Mốc đã khảo sát |
| --- | --- | --- |
| [MidnightTale/Line-GTK](https://github.com/MidnightTale/Line-GTK) — `master` | Bản gốc để đối chiếu | Nhánh cơ sở của các bản so sánh bên dưới |
| [shirafukayayoi/Line-GTK — main](https://github.com/shirafukayayoi/Line-GTK/tree/main) | Nhánh mặc định của fork, tập hợp các thay đổi phục vụ phát triển và PR lên bản gốc | `727124ff2d61841ff5cc11b48630e90da907c29e` |
| [shirafukayayoi/Line-GTK — personal/features](https://github.com/shirafukayayoi/Line-GTK/tree/personal/features) | Nhánh tính năng cá nhân, có nhiều bổ sung hơn `main` | `0907007bbe50d3fe4b87ca5968dd97c85c775a39` |

Tại thời điểm khảo sát, `main` đi trước bản gốc 17 commit, thay đổi 48 tệp; `personal/features` đi trước `main` thêm 55 commit. Những số liệu này là ảnh chụp tại thời điểm khảo sát, không phải trạng thái luôn cập nhật.

- [So sánh bản gốc với main của fork](https://github.com/MidnightTale/Line-GTK/compare/master...shirafukayayoi:main)
- [So sánh main với personal/features](https://github.com/shirafukayayoi/Line-GTK/compare/main...personal/features)
- [README của nhánh cá nhân tại mốc khảo sát](https://github.com/shirafukayayoi/Line-GTK/blob/0907007bbe50d3fe4b87ca5968dd97c85c775a39/README.md)

## Các bổ sung đã có trên main của fork

| Tính năng / sửa lỗi | Nội dung |
| --- | --- |
| Đăng nhập email và mật khẩu | Xác minh mã trên điện thoại khi cần; khôi phục phiên PC. Nhánh này không lưu mật khẩu. |
| Tiếng Nhật | Bản dịch giao diện, định dạng ngày giờ và phông mặc định Noto Sans CJK JP; giữ phông tùy chỉnh của người dùng. |
| Giải mã tin nhắn | Dùng linejs 3.5.0, xử lý nội dung `(none)`, hiển thị lỗi giải mã và thêm thao tác lấy lại khóa qua xác thực QR. |
| Thông tin nhóm | Lấy tên và ảnh nhóm theo từng đợt, lưu để dùng ở lần khởi động sau, giảm tình trạng hiển thị ID thay tên. |
| Tin nhắn xem trước | Cập nhật theo nội dung vừa lấy; tải nền toàn danh sách với tối đa hai yêu cầu đồng thời; kết thúc trạng thái đang tải khi không lấy được tin mới nhất. |
| Lịch sử cuộc gọi | Thẻ cuộc gọi đến, đi, nhỡ, không được trả lời; phân biệt thoại/video và hiển thị thời lượng khi có dữ liệu. |
| Trạng thái đã đọc | Hiển thị trạng thái gửi/đọc của tin nhắn, số người đã đọc trong nhóm và đồng bộ phạm vi đã đọc khi mở cuộc trò chuyện. |

Ngoài ra có cải thiện thời gian chờ xác thực, loại bỏ sự kiện đăng nhập muộn từ tiến trình cũ và dùng tiếng Anh cho từng mục còn thiếu bản dịch.

## Các bổ sung riêng trên personal/features

| Nhóm | Nội dung |
| --- | --- |
| Trả lời và thu hồi | Reply có trích dẫn bằng nút hoặc chuột phải; thu hồi tin đã gửi, nhận cập nhật thu hồi từ thiết bị khác và lưu trạng thái thu hồi. |
| Phân loại trò chuyện | Lọc Tất cả / Bạn bè / Nhóm / Tài khoản chính thức; tìm theo tên và tin nhắn xem trước trong loại đang chọn. |
| Ghim và ẩn | Ghim ở đầu danh sách, ẩn và hiện lại từ cài đặt; lưu riêng trên máy theo tài khoản. |
| Kiểm soát đã đọc | Dừng gửi đã đọc toàn bộ hoặc theo từng người; giữ chưa đọc; cho phép đánh dấu đã đọc đến thời điểm hiện tại mà vẫn giữ chế độ dừng. |
| Đồng bộ chưa đọc | Cập nhật badge và khay hệ thống khi người dùng đọc trên điện thoại hoặc thiết bị khác. |
| Chặn người dùng | Chặn/bỏ chặn trên máy chủ LINE, xem danh sách đang chặn và khóa gửi tin trong cuộc trò chuyện riêng bị chặn. |
| Thông tin người gửi trong nhóm | Hiện tên trên bong bóng tin nhắn, trong trích dẫn và thông báo hệ thống; cập nhật sau khi lấy được tên. |
| Sự kiện nhóm | Hiển thị mời, tham gia, rời nhóm và xóa thành viên bằng dòng thông báo có tên; lưu các sự kiện máy này nhận được. |
| Album | Xem album của một cuộc trò chuyện hoặc toàn bộ, phóng to và lưu ảnh gốc. |
| Emoji | Chọn/chèn/gửi emoji Unicode; hiển thị emoji riêng của LINE trong nội dung tin nhận. |
| Sticker | Xem trước ảnh tĩnh 160px khi rê chuột trong bộ chọn; hiển thị ảnh sticker trong thông báo hệ thống nếu được hỗ trợ. |
| Video và ảnh | Trình phát video có điều khiển và lưu video gốc; kiểm tra tính toàn vẹn, giải mã video Letter Sealing; cải thiện thanh công cụ xem và vẽ ảnh. |
| Phục hồi kết nối | Tạo lại kết nối nhận tin sau khi máy thức dậy hoặc mạng phục hồi, đồng bộ danh sách và tin gần đây, giữ bản nháp, trích dẫn và vị trí đọc. |
| Tự động đăng nhập khi khởi động | Tùy chọn lưu email/mật khẩu trong OS keyring qua Secret Service; ưu tiên phiên có sẵn và chỉ thử đăng nhập email khi cần. |
| Trải nghiệm nhập liệu | Tránh lấy mất tiêu điểm khỏi ô nhập khi danh sách trò chuyện cập nhật sau khi gửi. |

PR [#11 — Chat categories](https://github.com/MidnightTale/Line-GTK/pull/11) đã được khảo sát riêng. Tính năng này có trên `personal/features`, chưa nằm trong `main` tại mốc khảo sát. Không nên suy ra một tính năng đã có trên nhánh mặc định chỉ vì fork có nhánh `feature/*` hoặc `pr/*` tương ứng.

## Giới hạn được ghi nhận

- Gọi thoại vẫn được đánh dấu thử nghiệm/WIP. Đăng nhập email tạo phiên PC; tính năng gọi thoại thử nghiệm yêu cầu đăng nhập bằng QR.
- Album hỗ trợ xem và lưu ảnh; chưa tạo album, thêm/xóa ảnh hoặc xem video trong album.
- Emoji riêng của LINE mới hỗ trợ nhận và hiển thị ảnh tĩnh; chưa có danh sách/gửi emoji đã mua hoặc phát hoạt ảnh.
- Ghim và ẩn trò chuyện là thiết lập trên máy theo tài khoản, không đồng bộ trạng thái đó sang điện thoại.
- Sự kiện nhóm chỉ hiển thị phần máy này đã nhận và lưu; không phục dựng toàn bộ lịch sử thao tác trước đây.
- Tự động đăng nhập email trên nhánh cá nhân cần `secret-tool` và dịch vụ Secret Service, ví dụ GNOME Keyring.

## Định hướng cho repo này

Ưu tiên tham khảo fork của shirafukayayoi cho các đợt phát triển tiếp theo. Khi cập nhật tiếp, kiểm tra lại commit và diff mới nhất theo phạm vi cần dùng. Đợt tích hợp này đã nhận các thay đổi đến `0907007`; không đổi remote hoặc chuyển nhánh làm việc.

Các yêu cầu cần giữ khi tích hợp:

1. Giữ tiếng Việt và mặc định `vi` đã bổ sung trong repo này; fork tại mốc khảo sát chỉ có tiếng Thái, Anh và Nhật.
2. Bổ sung bản dịch tiếng Việt cho các khóa giao diện mới, giữ nguyên biến chèn và định dạng ngày giờ phù hợp.
3. Giải quyết cẩn thận phần cấu hình, chọn ngôn ngữ và phông chữ vì cả hai phía đều sửa các khu vực này.
4. Kiểm tra tương thích phiên đăng nhập, dữ liệu theo tài khoản, bộ nhớ đệm và phụ thuộc mới trước khi dùng dữ liệu thật.
5. Build và chạy kiểm thử Rust/Deno, rồi kiểm tra trực tiếp các luồng đăng nhập, nhắn tin, đã đọc, khôi phục kết nối và tính năng được chọn.

Đã tích hợp mã từ các mốc trên. Phần thông tin người gửi của fork hiện lấy tên thành viên; chưa có giao diện avatar cạnh từng tin nhắn trong nhóm. Phần ảnh nhóm dùng `picturePath` từ API nhóm và tải vào cache.

## Kiểm tra sau tích hợp

- `cargo test --all-targets --all-features`: 57 bài kiểm tra đạt.
- `deno task --config protocol/deno.json check`: định dạng, lint, kiểm tra kiểu và 144 bài kiểm tra đạt.
- Dependency cục bộ được nâng lên linejs/linejs-types 3.5.0, cùng mốc mà fork dùng để sửa giải mã.
- Các kết quả trên dùng kiểm tra tự động và dữ liệu giả lập; chưa xác nhận ảnh nhóm/tin nhắn trên tài khoản LINE thật.
