# Chuẩn bị poll đồ ăn

Chuột phải một **nhóm** trong danh sách → **Poll đồ ăn…**. Menu cũng mở bằng phím Menu hoặc Shift+F10.

- Đánh dấu nhóm, đặt tiêu đề và lưu giờ/ngày dự kiến cho nhóm đó.
- Mặc định: 10:00 thứ Hai–thứ Sáu, múi giờ Asia/Ho_Chi_Minh; tối đa 6 quán.
- Địa chỉ mặc định: **33 Dương Quang Đông, Quận 8, TP. Hồ Chí Minh, Việt Nam**. Bán kính cố định 5 km.
- Nhập Google Maps API key, rồi chọn **Tìm quán và xem bản nháp**. Có thể xem nháp trước khi lưu.
- Bản nháp cho biết địa chỉ Google xác định, tên quán, địa chỉ, khoảng cách và link Google Maps. Có thể chọn/copy nội dung bản nháp.

Google Cloud cần bật **Geocoding API** và **Places API (New)** cùng billing/quota phù hợp. Key nhập mới chỉ được lưu khi bấm **Lưu cấu hình nhóm**; để trống giữ key cũ. Key dùng chung giữa các nhóm của cùng tài khoản, lưu riêng trong `food-polls-<account>.json` ở thư mục dữ liệu ứng dụng với quyền 0600 trên Linux. Không đưa key vào kết quả RPC hay thông báo lỗi mạng.

Nguồn quán là [Google Places Nearby Search](https://developers.google.com/maps/documentation/places/web-service/nearby-search), tối đa 20 kết quả gần nhất trước khi chọn số quán đã cấu hình. Tọa độ trung tâm được tra bằng [Geocoding API](https://developers.google.com/maps/documentation/geocoding/requests-geocoding); địa chỉ mơ hồ hoặc partial match sẽ bị từ chối. Khoảng cách là đường chim bay, không phải quãng đường giao hàng. Loại quán ngoài 5 km, trùng ID hoặc không có trạng thái OPERATIONAL. Trạng thái này không xác nhận quán đang mở cửa hay giao hàng tới địa chỉ. Chưa tích hợp Grab.

**Phạm vi hiện tại:** chuẩn bị bản nháp và lưu cấu hình nhóm. Chưa có scheduler hay lệnh gửi/tạo poll; giờ/ngày lưu lại là cấu hình dự kiến. Không tự gửi tin nhắn vào nhóm. Thư viện LINE vendored hiện chưa có API poll gốc; cần chốt hình thức bình chọn (poll gốc LINE hoặc trả lời số và tổng hợp) để hoàn thiện bước đăng poll và job. Chưa có cơ chế chạy khi ứng dụng đóng.

Kiểm tra phần này:

```sh
cd protocol
deno test --allow-read --allow-write tests/food_polls_test.ts
deno check src/main.ts
```

Các kiểm tra dùng dữ liệu giả lập HTTP, không truy vấn Google và không gửi vào nhóm LINE thật.
