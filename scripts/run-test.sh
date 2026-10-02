#!/bin/bash
# Chạy trên máy Linux có cắm ZK9500. Giải nén ở đâu cũng được.
cd "$(dirname "$(readlink -f "$0")")" || exit 1
echo "== Thiết bị:"
lsusb | grep -i 1b55 || { echo "KHÔNG thấy đầu đọc 1b55 — cắm lại / gắn vào VM (VM > Removable Devices)."; exit 1; }
for d in /sys/bus/usb/devices/*; do
  [ "$(cat "$d/idVendor" 2>/dev/null)" = 1b55 ] || continue
  node=$(printf '/dev/bus/usb/%03d/%03d' "$(cat "$d/busnum")" "$(cat "$d/devnum")")
  if [ -w "$node" ]; then echo "Quyền ghi $node: OK"; else echo "CHƯA có quyền ghi $node -> chạy lại bằng: sudo $0"; exit 1; fi
done
if pgrep -x Bridge >/dev/null; then echo "Bridge đang chạy và có thể giữ đầu đọc -> bấm Thoát ở khay hệ thống rồi chạy lại."; exit 1; fi
echo
echo "== 1) zkprobe: thư viện Rust nói chuyện trực tiếp với ZK9500 (chưa qua SDK)"
./zkprobe 1 30 || { echo "zkprobe lỗi (mã $?)"; exit 1; }
echo
echo "== 2) zkfp-selftest: libzkfp.so của SDK + libsilkidcap.so mới (Init/Open/chụp/trích mẫu/ghép/so khớp)"
ZK9500CAP_DEBUG=1 LD_LIBRARY_PATH="$PWD" ./zkfp-selftest 60
