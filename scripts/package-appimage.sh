#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

tool=${APPIMAGETOOL:-"$PWD/target/appimagetool-x86_64.AppImage"}
binary=${DOCKSIE_BINARY:-"$PWD/target/release/docksie"}
runtime=${APPIMAGE_RUNTIME:-"$PWD/target/appimage-runtime-x86_64"}
if [[ ! -x "$tool" ]]; then
    mkdir -p "$(dirname "$tool")"
    curl --fail --location --retry 3 \
        'https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage' \
        --output "$tool"
    printf '%s  %s\n' ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 "$tool" | sha256sum --check
    chmod +x "$tool"
fi

if [[ ! -s "$runtime" ]]; then
    mkdir -p "$(dirname "$runtime")"
    curl --fail --location --retry 3 \
        'https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64' \
        --output "$runtime"
fi

mkdir -p target dist
appdir=$(mktemp -d "$PWD/target/docksie.AppDir.XXXXXX")
trap 'rm -r -- "$appdir"' EXIT
install -D -m 755 "$binary" "$appdir/usr/bin/docksie"
install -m 644 assets/io.docksie.app.desktop "$appdir/io.docksie.app.desktop"
install -m 644 assets/icon.png "$appdir/io.docksie.app.png"
ln -s io.docksie.app.png "$appdir/.DirIcon"
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
exec "$(dirname "$(readlink -f "$0")")/usr/bin/docksie" "$@"
APPRUN
chmod +x "$appdir/AppRun"

ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --no-appstream --runtime-file "$runtime" "$appdir" "$PWD/dist/Docksie-0.1.0-x86_64.AppImage"
