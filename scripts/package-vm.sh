#!/bin/bash
# Gói thử cho VM: /mnt/c/Users/tanmb/Downloads/zk9500-test.tar.gz
set -e
cd "$(dirname "$0")/.."
SDK="${SDK:-/mnt/c/Users/tanmb/Downloads/ZKFingerSDK 5.3_ZK10.0/ZKFingerSDK 5.3_ZK10.0/ZKFingerSDK 5.3_Linux_ZK10.0/SDK/lib-x64}"
P=$(mktemp -d)/zk9500-test
mkdir -p "$P"
for f in libzkfp.so libzkfinger10.so libidkit.so.2 libcrypto.so.0.9.8 libiomp5.so libusb-0.1.so.4; do cp "$SDK/$f" "$P/"; done
cp dist/libsilkidcap.so dist/zkprobe dist/zkfp-selftest scripts/run-test.sh "$P/"
chmod +x "$P"/zkprobe "$P"/zkfp-selftest "$P"/run-test.sh
for f in "$P"/libsilkidcap.so "$P"/zkprobe "$P"/zkfp-selftest; do echo "$(basename $f): $(objdump -T $f | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"; done
OUT=/mnt/c/Users/tanmb/Downloads/zk9500-test.tar.gz
tar -C "$(dirname "$P")" -czf "$OUT" zk9500-test
ls -la "$OUT"; tar -tzvf "$OUT"
