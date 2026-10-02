#!/bin/bash
# Không có đầu đọc trong WSL: chỉ kiểm tra libzkfp.so THẬT nạp được libsilkidcap.so của mình,
# Init báo "không có thiết bị" đúng mã (-2) và không crash.
set -e
cd "$(dirname "$0")/.."
SDK="${SDK:-/mnt/c/Users/tanmb/Downloads/ZKFingerSDK 5.3_ZK10.0/ZKFingerSDK 5.3_ZK10.0/ZKFingerSDK 5.3_Linux_ZK10.0/SDK/lib-x64}"
D=$(mktemp -d)
cp "$SDK"/*.so* "$D"/
cp dist/libsilkidcap.so "$D"/          # ghi đè bản gốc
cp dist/zkfp-selftest "$D"/
cd "$D"
echo "== ldd libzkfp.so (libsilkidcap phải trỏ vào $D):"
LD_LIBRARY_PATH=. ldd ./libzkfp.so | grep -E "silkid|zkfinger|usb-1|not found"
set +e
ZK9500CAP_DEBUG=1 LD_LIBRARY_PATH=. ./zkfp-selftest 3
echo "exit=$?"
rm -rf "$D"
