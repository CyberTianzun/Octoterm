#!/usr/bin/env bash
# 把 octoterm-desktop 组装成一个 .app。
# LSUIElement=1 是关键:没有它,一个托盘常驻程序会在 Dock 里留个图标、
# 还会接管顶部菜单栏。
set -euo pipefail

# 默认值跟随本机架构:写死 aarch64 会让 Intel Mac 上的报错建议指向错误的 target
case "$(uname -m)" in
  arm64) HOST_TARGET="aarch64-apple-darwin" ;;
  *)     HOST_TARGET="x86_64-apple-darwin" ;;
esac
TARGET="${1:-$HOST_TARGET}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/bundle/octoterm.app"
BIN="$ROOT/target/$TARGET/release/octoterm-desktop"
# 版本号只有一份事实来源:两处各写一个数字,升版本时必然忘掉其中一个
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/crates/desktop/Cargo.toml" | head -1)"
[ -n "$VERSION" ] || { echo "无法从 crates/desktop/Cargo.toml 读出 version" >&2; exit 1; }

[ -f "$BIN" ] || { echo "找不到 $BIN,先跑 cargo build --release --target $TARGET -p octoterm-desktop" >&2; exit 1; }

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/octoterm-desktop"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>octoterm</string>
    <key>CFBundleDisplayName</key>
    <string>octoterm</string>
    <key>CFBundleIdentifier</key>
    <string>com.octoterm.desktop</string>
    <key>CFBundleExecutable</key>
    <string>octoterm-desktop</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

echo "$APP"
