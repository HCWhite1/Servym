#!/usr/bin/env bash
# Retire de l'AppImage les bibliothèques Wayland embarquées par linuxdeploy. Construites sur
# Ubuntu 22.04, elles ne s'accordent pas avec le Mesa des distributions plus récentes (Fedora,
# Arch) : WebKitGTK s'arrête avec « Could not create default EGL display: EGL_BAD_PARAMETER » et la
# fenêtre reste vide. Tous les bureaux Linux ont leur propre libwayland.
#
#   tools/fix-appimage.sh target/release/bundle/appimage/Servym_<version>_amd64.AppImage
set -euo pipefail

image=$(realpath "$1")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

# Sans FUSE (CI, conteneurs) : les AppImage s'extraient au lieu de se monter.
export APPIMAGE_EXTRACT_AND_RUN=1
"$image" --appimage-extract >/dev/null
removed=$(find squashfs-root/usr/lib -maxdepth 1 -name 'libwayland-*' -print -delete | wc -l)
echo "libwayland retirées : $removed"

APPIMAGETOOL_VERSION=1.9.1
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
curl -fsSL -o appimagetool "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-x86_64.AppImage"
echo "${APPIMAGETOOL_SHA256}  appimagetool" | sha256sum -c -
chmod +x appimagetool
ARCH=x86_64 ./appimagetool --no-appstream squashfs-root "$image.new" >/dev/null
mv "$image.new" "$image"
echo "AppImage refaite : $image"
