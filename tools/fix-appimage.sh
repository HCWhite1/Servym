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

curl -fsSL -o appimagetool https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x appimagetool
ARCH=x86_64 ./appimagetool --no-appstream squashfs-root "$image.new" >/dev/null
mv "$image.new" "$image"
echo "AppImage refaite : $image"
