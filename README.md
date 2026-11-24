# untethered 

## Rust / Iced branch

Try the native Discord-style frontend from the checkout:

```sh
bash scripts/rust-client.sh       # Linux x86-64, current Rust toolchain and system GUI libraries
```

On x64 Windows with Rust and the MSVC build tools: `.\scripts\rust-client.ps1`.
Both launchers use gomuks **v0.2611.1** for Matrix and store this client's session in
`.dev/rust-profile`. The Rust and Qt sessions are independent. Set `UNTETHERED_BACKEND`
and `UNTETHERED_PROFILE` to use another compatible binary and dedicated profile.

The first launch opens a **local preview**: try the space rail, direct messages,
channel switching, People, Copy chat, or the composer (Enter sends, Shift+Enter adds
a line). Preview messages are never sent and disappear when you quit. Choose
**Sign in to Matrix** for live rooms. Enter a homeserver such as `matrix.org`;
the client adds `https://`. This prototype supports password login and verification
with the account’s existing recovery key (grouped or compact) or recovery passphrase.
The web verification form supports ordinary keyboard and context-menu paste.
SSO and first-time encryption setup aren’t implemented. Emoji comparison (SAS)
needs support from the client engine; the pinned gomuks release does not expose
that flow through its frontend API. Room sync, encryption, history, and sends use the upstream
gomuks engine. The UI currently renders plain text and initial-based avatars.

![Untethered room preview in the Rust frontend](rust-client/docs/preview.png)

### Browser build

The browser runs the same Rust view and an embedded gomuks WebAssembly worker.
No desktop gomuks process or proxy is needed. Its Matrix device lives in the
browser's origin-private storage, independently of the desktop profile. Use a
modern browser at `localhost` or an HTTPS origin.

```sh
rustup target add wasm32-unknown-unknown
# Requires Trunk, wasm-bindgen-cli matching Cargo.lock, Node/npm, Go and Git.
bash scripts/rust-web.sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory rust-client/web/dist
```

Open `http://127.0.0.1:8080`. The build uses the pinned upstream Go worker and
SQLite bridge. The native transport, browser transport, room model and Iced view
are in `rust-client/src`. `cargo test --manifest-path rust-client/Cargo.toml`
runs the model regression test; the ignored live test requires a disposable
loopback homeserver, an encrypted room and an isolated profile.

## Original Qt client

An independent Matrix desktop prototype with a Qt Quick interface and a local gomuks backend. It opens as a normal Linux app and stores the Matrix session under the current user's local application data directory.

## Build

Run `makepkg -s` in this directory. The recipe downloads the official x86-64 gomuks desktop release, checks its SHA-256 hash and packages only its gomuks backend alongside the Qt application.

The backend is [gomuks v0.2611.1](https://github.com/gomuks/gomuks/releases/tag/v0.2611.1), with [source at commit 5d21e2c](https://github.com/gomuks/gomuks/tree/5d21e2c7b31aabbba30efdd33f5882d99972e00e). Qt comes from system packages. The package is for local prototype testing.

## Develop in Code OSS

Open this project folder in Code OSS. Use **Terminal → Run Task → untethered: Run**
to build and launch the app. **Ctrl+Shift+B** builds it without launching.
The first run downloads and verifies the pinned gomuks backend. Later runs reuse it
and build the frontend incrementally. No package installation is required.

From a shell the equivalent is `bash scripts/dev.sh run`. The development
profile lives under `.dev/profile/untethered` and has a separate Matrix session.
The installed package and its profile can be used independently.

To uninstall the package, run `sudo pacman -R untethered`. Pacman removes
the installed app and backend but keeps your Matrix session and encryption keys
under `~/.local/share/untethered` (or your configured `XDG_DATA_HOME`).


## Local startup test

Run `untethered --smoke-test` to test Qt startup and the backend handshake. Set `XDG_DATA_HOME` to a fresh directory for an isolated profile. Developers can pass `--backend /absolute/path/to/gomuks` when running an unpackaged build.
