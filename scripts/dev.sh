#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
root="$PWD"
dev="$root/.dev"
mode="${1:-run}"
if [[ "$mode" != build && "$mode" != run ]]; then
    printf 'Usage: %s [build|run [app arguments]]\n' "$0" >&2
    exit 2
fi
if (( $# )); then shift; fi

mkdir -p "$dev/build"
(
    cd "$dev/build"
    /usr/lib/qt6/bin/qmake "$root/untethered.pro"
    make -j "${UNTETHERED_BUILD_JOBS:-2}"
)
if [[ "$mode" == build ]]; then exit 0; fi

# Use the same pinned upstream version and checksum as the CachyOS package.
source "$root/PKGBUILD"
backend="$dev/backends/$_gomuks_release/gomuks"
if [[ ! -x "$backend" ]]; then
    mkdir -p "$(dirname -- "$backend")" "$dev/downloads"
    archive="$dev/downloads/$_deb"
    url="${source[-1]#*::}"
    expected="${sha256sums[-1]}"
    tmp=$(mktemp -d "$dev/downloads/prepare.XXXXXX")
    trap 'rm -f -- "$tmp/$_deb" "$tmp/gomuks"; rmdir -- "$tmp"' EXIT
    if [[ ! -f "$archive" ]]; then
        curl --fail --location --silent --show-error "$url" --output "$tmp/$_deb"
        printf '%s  %s\n' "$expected" "$tmp/$_deb" | sha256sum -c -
        mv -- "$tmp/$_deb" "$archive"
    else
        printf '%s  %s\n' "$expected" "$archive" | sha256sum -c -
    fi
    ar p "$archive" data.tar.xz | bsdtar -xOf - ./usr/lib/gomuks-desktop/resources/gomuks > "$tmp/gomuks"
    chmod 755 "$tmp/gomuks"
    mv -- "$tmp/gomuks" "$backend"
    rmdir -- "$tmp"
    trap - EXIT
fi

export XDG_DATA_HOME="$dev/profile"
exec "$dev/build/untethered" --backend "$backend" "$@"
