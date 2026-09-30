#!/bin/sh
# Build "P1505 手動雙面.app" (Apple silicon, ad-hoc signed) into build/.
#   ./build.sh            build
#   ./build.sh --install  also copy it to /Applications and launch it
set -e
cd "$(dirname "$0")"

swift build -c release --arch arm64
BIN=$(swift build -c release --arch arm64 --show-bin-path)/P1505Duplex

APP="build/P1505 手動雙面.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/P1505Duplex"
cp Resources/Info.plist "$APP/Contents/Info.plist"
swift scripts/make-icon.swift "$APP/Contents/Resources/AppIcon.icns"
codesign --force --sign - --options runtime --timestamp=none "$APP"
codesign --verify --strict "$APP"
echo "built $APP"

if [ "$1" = "--install" ]; then
    pkill -x P1505Duplex 2>/dev/null || true
    # Wait for the old instance to exit, or `open` may fail with -600.
    while pgrep -x P1505Duplex >/dev/null; do sleep 0.2; done
    rm -rf "/Applications/P1505 手動雙面.app"
    cp -R "$APP" /Applications/
    # LaunchServices can still be tearing down the old instance (-600);
    # retry for a few seconds.
    for _ in 1 2 3 4 5; do
        open "/Applications/P1505 手動雙面.app" 2>/dev/null && break
        sleep 1
    done
    echo "installed to /Applications"
fi
