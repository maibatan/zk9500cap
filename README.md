# zk9500cap

`libsilkidcap.so` thay thế cho **ZKFinger SDK 5.3 Linux (ZK10.0)**, để đầu đọc vân tay
**ZKTeco ZK9500 (USB `1b55:0124`)** chạy được trên Linux.

`libsilkidcap.so` gốc trong SDK chỉ nhận cứng `1b55:0120`. Nó đọc EEPROM của ZK9500 bằng
những lệnh mà ZK9500 trả về 0 byte, nên `ZKFPM_OpenDevice` luôn trả NULL. Thư viện này giữ
nguyên ABI mà `libzkfp.so` gọi, nhưng nói giao thức của ZK9500. `libzkfp.so` và
`libzkfinger10.so` (thuật toán) **giữ nguyên**, nên mẫu vân tay vẫn là chuẩn ZK10 và tương
thích với mẫu tạo trên Windows.

## ABI (lấy từ objdump libzkfp.so / libsilkidcap.so gốc)

| Hàm | Ghi chú |
|---|---|
| `int sensorInit(void)` | 0 nếu có đầu đọc, -19 nếu không |
| `int sensorGetCount(void)` | |
| `int sensorFree(void)` | |
| `void* sensorOpen(int index)` | NULL nếu lỗi |
| `int sensorClose(void* h)` | |
| `int sensorCapture(void* h, u8* buf, int size)` | không chặn chờ; >0 = số byte ảnh, 0 = chưa có tay. `size` là rác khi gọi từ `AcquireTemplate`, nên bỏ qua |
| `int sensorGetParameter(void* h, int code)` | 1 rộng, 2 cao, 3 dpi, 106 rộng×cao, 1015 VID\|PID<<16 |
| `int sensorGetParameterEx(h, code, buf, int* size)` | số nguyên 4 byte; 1101 hãng, 1102 model, 1103 serial, 1104 custom 16 byte |
| `int sensorSetParameterEx(h, code, buf, size)` | nhận rồi bỏ qua |
| `int sensorCheckLic(h, u32 challenge, void*)` | callback bản quyền của thuật toán, chuyển xuống firmware |

## Giao thức ZK9500

Giải mã từ USBPcap khi chạy `libzkfp.dll` trên Windows. Mọi lệnh là vendor control trên EP0,
riêng ảnh đi qua bulk IN `0x82`.

| Bước | Lệnh |
|---|---|
| Mở | `SET_CONFIGURATION 1`, claim if 0, OUT `0xE0`, OUT `0x80` + 16 byte 0, bulk IN 4 byte |
| EEPROM | IN `0xE7` wIndex=địa chỉ, 1 byte mỗi lần. `0x30..0x3b` = exposure, gain R/G1/G2/B, checksum (u16 LE); `0x70..0x75` = LED Main1/2, Side1/2, Anti1/2 |
| LED | OUT `0xE1` wValue=`(mức<<8)\|1` wIndex=`0..5` |
| CMOS | OUT `0xE3` wValue=gain, wIndex=`a5 a6`(R) `a3 a4`(G1) `a9 aa`(G2) `a7 a8`(B); exposure: wIndex 3 = byte cao, 4 = byte thấp |
| Bật chụp | OUT `0xE1`: (1, `0x53`), (0, `0x32`), (1, `0x31`), (3, `0x30`) |
| Có tay? | IN `0xEA` 1 byte: `01` = có ngón tay, ảnh đang chờ trên EP `0x82` |
| Ảnh | bulk IN `0x82`, 112500 byte = 300×375 ảnh xám 8 bit. Đây đã là ảnh cuối, trùng từng byte với ảnh `libzkfp.dll` đưa cho thuật toán |
| Bản quyền | IN `0xF3` wValue=challenge>>16 wIndex=challenge&0xffff, 4 byte, u32 LE |
| Đóng | OUT `0xE1` wValue 0, wIndex `0x15`, `0x30`, `0x31` |

## Build (WSL Ubuntu)

```bash
scripts/build.sh        # dist/libsilkidcap.so, dist/zkprobe, dist/zkfp-selftest + unit test
scripts/wsl-smoke.sh    # libzkfp.so thật + thư viện này, không cần thiết bị (Init phải ra -2)
scripts/package-vm.sh   # ~/Downloads/zk9500-test.tar.gz để thử trên máy có đầu đọc
```

Thư viện link động `libusb-1.0.so.0` của hệ thống và cần **glibc ≥ 2.34** (Ubuntu 22.04+).
Không dùng feature `vendored-libusb` cho bản phát hành: build trên glibc ≥ 2.38 thì libusb kéo
theo `__isoc23_*` (GLIBC_2.38).

Bật log chẩn đoán bằng biến môi trường `ZK9500CAP_DEBUG=1` (ghi ra stderr).
