# Untethered

Untethered’s frontend is written in Rust using Iced. The Matrix engine is the
upstream [gomuks](https://github.com/gomuks/gomuks) project, written in Go.

See the [Discord-parity roadmap and milestone checks](ROADMAP.md).

## Desktop

Try the native Discord-style frontend from the checkout:

```sh
bash scripts/rust-client.sh       # Linux x86-64, current Rust toolchain and system GUI libraries
```

On x64 Windows with Rust and the MSVC build tools: `.\scripts\rust-client.ps1`.
Both launchers use gomuks **v0.2611.1** for Matrix and store this client's session in
`.dev/rust-profile`. Set `UNTETHERED_BACKEND`
and `UNTETHERED_PROFILE` to use another compatible binary and dedicated profile.

**How startup works:** The dev launcher fetches the pinned gomuks executable
on first use, verifies its SHA-256 and caches it in `.dev/backends/`. On every
launch, the Rust app locks the profile, starts `gomuks --desktop` as a child,
and reads its loopback address from stdout. It authenticates with a fresh
per-launch secret and opens a local WebSocket for Matrix commands and events.
Closing the app shuts down the child. There’s no separate service to start.

In Code OSS, **Ctrl+Shift+B** builds the Rust binary; **Terminal → Run Task →
untethered: Run** launches the desktop app.

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

**Files:** Drop files onto a room or click **+** to stage them, optionally type
a caption for the first attachment, then press **Send**. A local preview strip
shows image thumbnails or file-type cards; use **×** to remove a file. PNG,
JPEG, GIF and WebP previews are static. Oversized or unsupported images still
send as ordinary attachments within the upload limit. Each file is a standard
Matrix media event. Draft attachments stay in their original room when you switch.
Uploads are sequential, with up to 10 staged files per room and a 64 MiB/file cap
(lower if the homeserver advertises a smaller limit). Click **Save** on a received
attachment. Preview mode simulates a send locally; it never uploads.

During upload the card shows the transfer percentage and a **Cancel** button.
Cancel stops the active upload and pauses the batch, keeping the files and
caption in the original room. **Retry** resumes the unsent files; files already
confirmed in the room are not sent again. Once an attachment is sending its
room event, it can no longer be cancelled as an upload.

To send a screenshot, copy it and press **Ctrl+V in the message composer**.
An image becomes a removable PNG attachment; plain text is inserted at the cursor.
The image stays local until Send. Linux Wayland reads the focused clipboard
through the app's existing data device; no clipboard watcher or helper daemon is needed.

Received images show an inline thumbnail in the active conversation. The client
uses encrypted Matrix thumbnails when available, or decodes a small original
(up to 4 MiB) via gomuks. **Save** still downloads the original. The repeatable
`scripts/test-media-interop.py` test checks actual pixels in an independent browser
device after both login and reload against a disposable Synapse.

**Calls (experimental):** Open your account via the bottom-left profile button and
choose **Check call server support** to query gomuks for MatrixRTC transports.
This checks discovery only; it doesn't ring anyone or open a microphone.

![Untethered room preview in the Rust frontend](rust-client/docs/preview.png)

## Browser

The browser runs the same Rust view and an embedded gomuks WebAssembly worker.
No desktop gomuks process or proxy is needed. Its Matrix device lives in the
browser's origin-private storage, independently of the desktop profile. Use a
modern browser at `localhost` or an HTTPS origin.

Only one tab can use the browser device at once. Another tab waits and connects
automatically when the active tab closes. If upgrading from an older build that
reported “worker stopped,” close the old tabs and reopen the same URL. Keep the
site data: that’s where the device’s encryption keys live.

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

## Arch Linux package (x86-64)

Run `makepkg -s` in the checkout to build the **current local Git commit**.
The package installs `/usr/bin/untethered` and `/usr/lib/untethered/gomuks`.
Launching `untethered` uses that bundled backend without a runtime download.
The packaged app keeps its Matrix device in `$XDG_DATA_HOME/untethered-iced`
(default `~/.local/share/untethered-iced`), separate from the development profile.
