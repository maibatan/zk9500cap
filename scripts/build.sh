#!/bin/bash
# Build + kiểm tra trong WSL/Linux. Kết quả: dist/libsilkidcap.so, dist/zkprobe
set -e
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/zk9500cap-target}"
cargo build --release --offline 2>&1 | grep -E "^(warning|error)|Finished" || true
cargo test --release --offline --lib 2>&1 | grep -E "^test |test result"
T="$CARGO_TARGET_DIR/release"
mkdir -p dist
cp "$T/libsilkidcap.so" "$T/zkprobe" dist/
ls -la dist
echo "== sensor* xuat ra:"; objdump -T dist/libsilkidcap.so | awk '/ sensor/{print $NF}' | sort | tr '\n' ' '; echo
echo "== GLIBC cao nhat can:"; objdump -T dist/libsilkidcap.so | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -3 | tr '\n' ' '; echo
echo "== NEEDED:"; objdump -p dist/libsilkidcap.so | awk '/NEEDED/{print $2}' | tr '\n' ' '; echo
gcc -O2 -Wall -o dist/zkfp-selftest tools/zkfp-selftest.c -ldl
echo "== selftest GLIBC:"; objdump -T dist/zkfp-selftest | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
