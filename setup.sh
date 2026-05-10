#!/bin/zsh

set -e -o pipefail

BASEDIR=$(dirname "$0")
APP="/Applications/LibraryLoader.app"

echo "==> Checking dependencies..."
BREW=$(which brew || { echo "Homebrew not found. Install it from https://brew.sh"; exit 1; })
which rustc > /dev/null || { echo "Rust not found. Install it from https://rustup.rs"; exit 1; }
CARGO=$(which cargo)

REQUIRED_PKG=("gtk+3" "atk" "gdk-pixbuf" "pango" "adwaita-icon-theme" "jpeg")
for PKG in $REQUIRED_PKG; do
    $BREW ls --versions $PKG > /dev/null 2>&1 || $BREW install $PKG
done

echo "==> Building Library Loader..."
if [ $(uname -m) = "arm64" ]; then
    TARGET="aarch64-apple-darwin"
else
    TARGET="x86_64-apple-darwin"
fi
$CARGO build --release --target=$TARGET

BINARY="$BASEDIR/target/$TARGET/release/library-loader-gui"

echo "==> Creating app bundle..."
mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"

cp "$BINARY" "$APP/Contents/MacOS/library-loader-gui"

# Convert icon if rsvg-convert is available
if which rsvg-convert > /dev/null 2>&1; then
    ICONSET=$(mktemp -d)
    for size in 16 32 64 128 256 512; do
        rsvg-convert -w $size -h $size "$BASEDIR/ll-gui/assets/library-loader-icon.svg" -o "$ICONSET/icon_${size}x${size}.png"
        rsvg-convert -w $((size*2)) -h $((size*2)) "$BASEDIR/ll-gui/assets/library-loader-icon.svg" -o "$ICONSET/icon_${size}x${size}@2x.png"
    done
    mv "$ICONSET" /tmp/LibraryLoader.iconset
    iconutil -c icns /tmp/LibraryLoader.iconset -o "$APP/Contents/Resources/AppIcon.icns"
    rm -rf /tmp/LibraryLoader.iconset
fi

cat > "$APP/Contents/MacOS/LibraryLoader" << 'EOF'
#!/bin/bash
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin:$PATH"
export DYLD_FALLBACK_LIBRARY_PATH="/opt/homebrew/lib"
export XDG_DATA_DIRS="/opt/homebrew/share"
export GTK_DATA_PREFIX="/opt/homebrew"
export GTK_EXE_PREFIX="/opt/homebrew"
export GTK_PATH="/opt/homebrew"
export GDK_PIXBUF_MODULE_FILE="/opt/homebrew/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/library-loader-gui"
EOF

cat > "$APP/Contents/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>LibraryLoader</string>
    <key>CFBundleDisplayName</key><string>Library Loader</string>
    <key>CFBundleIdentifier</key><string>dev.itsnot.library-loader</string>
    <key>CFBundleVersion</key><string>0.5.0</string>
    <key>CFBundleExecutable</key><string>LibraryLoader</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict>
</plist>
EOF

chmod +x "$APP/Contents/MacOS/LibraryLoader" "$APP/Contents/MacOS/library-loader-gui"

echo ""
echo "Done! Library Loader is installed in /Applications."
echo "If macOS blocks it on first launch, right-click the app -> Open -> Open."
