# KWin / Dolphin ↔ Element file round trip

This is a Linux runtime test. It starts an actual KWin virtual output with
OpenGL compositing, its own session bus, Dolphin and a fresh native Matrix
profile. It does not start XWayland, use the primary compositor's socket,
or inject input into the user's desktop. It gives Element Web a **different
Matrix account** in a fresh Chromium profile, then invites that user into
the encrypted room. All services and files are disposable.

Prerequisites: `kwin_wayland` with virtual OpenGL output support, `dolphin`,
`spectacle`, `kdotool`, `qdbus6`, KDE portal/file chooser, `tesseract`, Chromium,
and KDE's `selenium-webdriver-at-spi-inputsynth` with its matching installed
desktop file. `KWIN_INPUTSYNTH` can point to a **private installation prefix**;
the fixture adds its `share` directory to the test's `XDG_DATA_DIRS`.
Tested with KWin 6.8.3, Dolphin 26.08.3, and KDE inputsynth built at
`4199d3ea3ccdcaa175b5a06ed3ad8f1af498f38a`. These are test tools, not
Untethered dependencies.

Set `ELEMENT_WEB_DIR` to the extracted official `element-v1.12.35.tar.gz`
(SHA-256 `5aaff32e348eba976aea8da11cd50f9ca7f824f235aa66e24d1176f829e6bba5`).
The fixture serves the release locally with its own config, without changing
the extracted files. Build the native debug client first and set
`UNTETHERED_BACKEND` if the pinned gomuks binary isn't already in `.dev/backends`.

```sh
cargo build --locked -j2 --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow --with cryptography \
    python scripts/test-media-interop.py --kwin-element
```

The test selects a PNG and PDF together in Dolphin and physically drags them
into Untethered, checking the Wayland transfer request and staged composer
cards. It checks that nothing was uploaded before Send, switches rooms while
the first upload is held, and compares Element's decrypted timeline event IDs
with the homeserver's encrypted events. It clicks Element's download controls,
compares both files, then uploads those downloaded bytes back under new names.
Finally it uses **Save attachment** in the native timeline and the KDE file
chooser, compares both downloads byte-for-byte, and checks sender, room, and
event counts. `KWIN_QA_DEBUG` optionally names a directory for test-only
compositor screenshots and diagnostic logs.
