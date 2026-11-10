#!/bin/sh
# Packs the native release build into a portable onelf bundle in dist/.
#
# Usage: scripts/package-onelf.sh <platform> <version>
#   scripts/package-onelf.sh x86_64-linux 0.1.0
#
# Produces dist/<app>-<platform>.onelf. Run `cargo build --release
# --locked` first. The bundle carries glibc and every library the binary
# loads, all from the build host. The GL stack is the host's: the app falls
# back to its own CPU renderer when the host has none, so Mesa is not bundled.
#
# The bundle updates itself from the same asset of the app's latest GitHub
# release, downloading only what changed. With the app's update key,
# packaging/onelf/update.pub, it accepts only releases the release
# workflow signed; without one it updates unsigned, and onelf warns about
# that on every update. See the release skill.
set -eu
cd "$(dirname "$0")/.."

platform=$1
version=$2
app=$(sed -n 's/^name = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
appdir=dist/AppDir

rm -rf "$appdir"
mkdir -p "$appdir/bin" "$appdir/.onelf/icons" "$appdir/.onelf/desktop"
cp "target/release/$app" "$appdir/bin/$app"
cp packaging/linux/app.desktop "$appdir/.onelf/desktop/default.desktop"

# A vector icon goes in as its SVG. A pixel-art one goes in as a raster,
# which onelf installs at its own size beside the bundled theme sizes, so
# a desktop scales the nearest of those as little as it can.
if [ -f assets/icon.svg ]; then
    cp assets/icon.svg "$appdir/.onelf/icons/default.svg"
    mkdir -p "$appdir/share/icons/hicolor/scalable/apps"
    cp assets/icon.svg "$appdir/share/icons/hicolor/scalable/apps/$app.svg"
else
    cp assets/icons/256.png "$appdir/.onelf/icons/default.png"
fi
for size in 16 24 32 48 64 128 256 512; do
    dir=$appdir/share/icons/hicolor/${size}x$size/apps
    mkdir -p "$dir"
    cp "assets/icons/$size.png" "$dir/$app.png"
done
sed "s/^version = .*/version = \"$version\"/" onelf.toml > "$appdir/onelf.toml"

if grep -q '^\[update\]' onelf.toml; then
    echo "onelf.toml has its own [update]; packaging adds it" >&2
    exit 1
fi
repo=$(sed -n 's|^repository = "https://github.com/\(.*\)"$|\1|p' Cargo.toml | head -n 1)
if [ -z "$repo" ]; then
    echo "Cargo.toml names no github.com repository to update from" >&2
    exit 1
fi
# The runtime installs only a strictly newer revision, so a client is never
# rolled back: 1.2.3 is 1002003.
core=${version%%-*}
major=${core%%.*}
rest=${core#*.}
minor=${rest%%.*}
patch=${rest#*.}
if [ "$minor" -ge 1000 ] || [ "$patch" -ge 1000 ]; then
    echo "version $version does not fit an update revision" >&2
    exit 1
fi
key=packaging/onelf/update.pub
if [ -f "$key" ]; then
    trust="key = \"$PWD/$key\""
else
    trust="unsigned = true"
fi
cat >> "$appdir/onelf.toml" <<EOF

[update]
url = "https://github.com/$repo/releases/latest/download/$app-$platform.onelf"
revision = $((major * 1000000 + minor * 1000 + patch))
$trust
EOF

onelf build "$appdir"
onelf verify "$appdir/$app.onelf"
mv "$appdir/$app.onelf" "dist/$app-$platform.onelf"
rm -rf "$appdir"
ls "dist/$app-$platform.onelf"
