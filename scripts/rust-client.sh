#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
source PKGBUILD
export UNTETHERED_PROFILE="${UNTETHERED_PROFILE:-$PWD/.dev/rust-profile}"
if [[ -z "${UNTETHERED_BACKEND:-}" ]]; then
    # Reuse the checksum-verified binary from the Qt dev runner when available.
    backend="$PWD/.dev/backends/$_gomuks_release/gomuks"
    if [[ ! -x "$backend" ]]; then
        [[ $(uname -sm) == 'Linux x86_64' ]] || { echo 'Set UNTETHERED_BACKEND to the gomuks executable.' >&2; exit 1; }
        mkdir -p "$(dirname "$backend")"
        tmp=$(mktemp "${backend}.XXXXXX")
        trap 'rm -f -- "$tmp"' EXIT
        curl -fsSL "https://github.com/gomuks/gomuks/releases/download/v$_gomuks_release/gomuks-amd64" -o "$tmp"
        echo "d17f1945befd9282f37600cd4343b0c823d56ea3e910cbaf177403426f0dd9e4  $tmp" | sha256sum -c -
        chmod 755 "$tmp"
        mv "$tmp" "$backend"
    fi
    export UNTETHERED_BACKEND="$backend"
fi
exec cargo run --locked -j "${UNTETHERED_BUILD_JOBS:-3}" --manifest-path rust-client/Cargo.toml -- "$@"
