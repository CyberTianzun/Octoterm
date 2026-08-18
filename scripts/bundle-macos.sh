#!/usr/bin/env bash
# 把 octoterm-desktop 组装成一个 .app。
# LSUIElement=1 是关键:没有它,一个托盘常驻程序会在 Dock 里留个图标、
# 还会接管顶部菜单栏。
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/bundle/octoterm.app"
BIN="$ROOT/target/$TARGET/release/octoterm-desktop"

[ -f "$BIN" ] || { echo "找不到 $BIN,先跑 cargo build --release --target $TARGET -p octoterm-desktop" >&2; exit 1; }

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/octoterm-desktop"

cat > "$APP/Contents/Info.plist" <<'PLIST'
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
    <string>0.1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

echo "$APP"
