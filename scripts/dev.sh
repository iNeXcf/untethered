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

# Use the pinned upstream artifacts from the package recipe.
source "$root/PKGBUILD"
if [[ -z "${LIVEKIT_SDK:-}" ]]; then
    export LIVEKIT_SDK="$dev/backends/$_livekit_dir"
    if [[ ! -f "$LIVEKIT_SDK/lib/liblivekit.so" || ! -f "$LIVEKIT_SDK/lib/liblivekit_ffi.so" ]]; then
        mkdir -p "$dev/downloads" "$dev/backends"
        (
            set -e
            scratch=$(mktemp -d "$dev/downloads/livekit.XXXXXX")
            trap 'rm -rf -- "$scratch"' EXIT
            archive="$dev/downloads/$_livekit_dir.tar.gz"
            if [[ ! -f "$archive" ]]; then
                curl --fail --location --silent --show-error "$_livekit_url" --output "$scratch/sdk.tar.gz"
                printf '%s  %s\n' "$_livekit_sha256" "$scratch/sdk.tar.gz" | sha256sum -c -
                mv -- "$scratch/sdk.tar.gz" "$archive"
            else
                printf '%s  %s\n' "$_livekit_sha256" "$archive" | sha256sum -c -
            fi
            tar -xzf "$archive" -C "$scratch"
            mv -T -- "$scratch/$_livekit_dir" "$LIVEKIT_SDK"
        )
    fi
fi

mkdir -p "$dev/build"
(
    cd "$dev/build"
    /usr/lib/qt6/bin/qmake "$root/untethered.pro"
    make -j "${UNTETHERED_BUILD_JOBS:-2}"
)
if [[ "$mode" == build ]]; then exit 0; fi

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
