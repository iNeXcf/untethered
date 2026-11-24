pkgname=untethered
pkgver=0.1.0
pkgrel=3
pkgdesc='Qt Quick desktop prototype for Matrix using a bundled gomuks backend'
arch=('x86_64')
conflicts=('gomuks-native' 'unteathered')
replaces=('gomuks-native' 'unteathered')
url='https://github.com/gomuks/gomuks'
license=('AGPL-3.0-or-later')
depends=('qt6-base' 'qt6-declarative' 'qt6-websockets' 'qt6-webengine' 'qt6-imageformats' 'ca-certificates')
optdepends=('qt6-wayland: Wayland platform plugin')
makedepends=('binutils')
_gomuks_release=0.2611.1
_deb="gomuks-desktop-v${_gomuks_release}-linux-amd64.deb"
source=('untethered.pro' 'main.cpp' 'Main.qml' 'ui.qrc' 'untethered.desktop' 'LICENSE'
        "${_deb}::https://github.com/gomuks/gomuks/releases/download/v${_gomuks_release}/gomuks-desktop-linux-amd64.deb")
noextract=("$_deb")
options=('!strip' '!debug')
sha256sums=(
    'b8b7ae958ae29507bd04a04ee0462711287a2db3d186c799d699ee7edcc7755c'
    '22943f10283df1bd688869345838cc96fea8b50bbd73e7def45bdd16b2cd9fc9'
    'e79b92f00be318675a4bd1ff2b8555794f8588e5f38aad920a61abd80bd68e87'
    '288315478e8401b8efe16eefa0568e2be0c199bb4f9876f01d3eb7a12c77f5dd'
    '636e7f40fa3f8031264be7c0ea51d265cd13f6c48a829f09f1ae006161c8ca3b'
    '0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0'
    '86a4f78b0cfd31de0c7530e37ee8462527a11ab5aebd8fe2e6cd778bf3345fa7'
)

build() {
    /usr/lib/qt6/bin/qmake untethered.pro
    make
}

package() {
    local backend_path=./usr/lib/gomuks-desktop/resources/gomuks
    install -Dm755 untethered "$pkgdir/usr/bin/untethered"
    install -Dm644 untethered.desktop "$pkgdir/usr/share/applications/untethered.desktop"
    ar p "$_deb" data.tar.xz | bsdtar -xOf - "$backend_path" > gomuks-backend
    install -Dm755 gomuks-backend "$pkgdir/usr/lib/untethered/gomuks"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
