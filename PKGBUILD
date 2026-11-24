# Local package: build the current checkout's committed Rust frontend.
pkgname=untethered
pkgver=0.1.0
pkgrel=3
pkgdesc='Native Rust/Iced Matrix client powered by gomuks'
arch=('x86_64')
url='https://github.com/koalexbear/untethered'
license=('AGPL-3.0-or-later' 'OFL-1.1' 'MIT')
optdepends=('xdg-desktop-portal: file picker with a desktop-specific portal backend')
depends=('ca-certificates' 'fontconfig' 'gcc-libs' 'glibc' 'libx11' 'libxcb'
         'libxcursor' 'libxi' 'libxkbcommon-x11' 'libxrandr' 'wayland')
makedepends=('cargo' 'git')
options=('!debug' '!lto')

source "$startdir/scripts/gomuks.env"
source=("untethered-src::git+file://$startdir#commit=$(git -C "$startdir" rev-parse HEAD)"
        "gomuks-$GOMUKS_VERSION-linux-amd64::https://github.com/gomuks/gomuks/releases/download/v$GOMUKS_VERSION/gomuks-amd64")
sha256sums=('SKIP' "$GOMUKS_LINUX_SHA256")

build() {
    cd "$srcdir/untethered-src/rust-client"
    cargo build --release --locked --target-dir "$srcdir/target" -j "${UNTETHERED_BUILD_JOBS:-3}"
}

check() {
    cd "$srcdir/untethered-src/rust-client"
    cargo test --release --locked --target-dir "$srcdir/target" -j "${UNTETHERED_BUILD_JOBS:-3}"
}

package() {
    cd "$srcdir/untethered-src"
    install -Dm755 "$srcdir/target/release/untethered-iced" "$pkgdir/usr/bin/untethered"
    install -Dm755 "$srcdir/gomuks-$GOMUKS_VERSION-linux-amd64" "$pkgdir/usr/lib/untethered/gomuks"
    install -Dm644 untethered.desktop "$pkgdir/usr/share/applications/untethered.desktop"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/untethered/LICENSE"
    install -Dm644 rust-client/assets/LICENSE "$pkgdir/usr/share/licenses/untethered/Liberation-fonts"
    install -Dm644 rust-client/vendor/smithay-clipboard/LICENSE "$pkgdir/usr/share/licenses/untethered/smithay-clipboard"
}
