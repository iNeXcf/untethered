#!/usr/bin/env bash
# Build the same Iced UI with the upstream gomuks browser worker.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
root="$PWD"
src="$root/.dev/gomuks-src"
stage="$root/.dev/wasm-worker"
commit=5d21e2c7b31aabbba30efdd33f5882d99972e00e
mkdir -p "$stage"
if [[ ! -d "$src" ]]; then
    git clone --depth=1 --branch v0.2611.1 https://github.com/gomuks/gomuks.git "$src"
fi
[[ $(git -C "$src" rev-parse HEAD) == "$commit" ]] || { echo 'Unexpected gomuks checkout' >&2; exit 1; }
(
    cd "$src/web"
    GOMAXPROCS=3 GOFLAGS=-p=3 sh build-wasm.sh
)
cp "$src/web/src/api/wasm/"{wasmuks.ts,sqlite_bridge.ts,_gomuks.wasm} "$stage/"
cp "$(go env GOROOT)/lib/wasm/wasm_exec.js" "$stage/go_wasm_exec.js"
cp rust-client/web/worker/{package*.json,entry.js,profile-lock.js,vite.config.js} "$stage/"
(
    cd "$stage"
    patch --batch --fuzz=0 -p1 < "$root/rust-client/web/worker/startup.patch"
    npm ci --no-audit --no-fund
    ./node_modules/.bin/vite build
)
cd rust-client/web
CARGO_BUILD_JOBS=3 trunk build --no-sri "$@"
cp -R "$stage/dist" dist/backend
