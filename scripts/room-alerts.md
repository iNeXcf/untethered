# Verify per-room Linux desktop notifications

Use the native debug build and [Element test environment](replies-element.md),
plus `dbus-daemon`, `dbus-monitor`, `busctl`, `xmessage`, Dunst and its sibling
`dunstctl`. Set `DUNST` if the binary isn’t on `PATH`. From the checkout:

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 DUNST=/path/to/dunst \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --room-alerts
```

The test runs Dunst on its own Xvfb display and session bus and checks the
bus owner PID. It creates two verified disposable accounts on a loopback
Synapse. Element sends Megolm messages with explicit `m.mentions`; the
receiver must actually decrypt each one. A separate window holds keyboard
focus during the background-delivery checks.

| Receiver setting and event | Expected desktop result |
|---|---|
| **All messages**, ordinary message | One visible popup |
| **Mute ordinary**, ordinary message or mention of another user | No `Notify` call |
| Same muted room, direct mention of the receiver | One visible popup |
| Untouched control room, ordinary message | One visible popup |
| Restarted client, mention in the focused room | No popup or history replay |
| Restarted client, backgrounded muted room | No popup for ordinary chat; one for a direct mention |

The fixture reads each positive popup on screen, checks the app name and
body in Dunst’s history, and counts the desktop service calls, including
the negative controls. It also reads the receiver’s room push rule from
Synapse and verifies **Account default** deletes that override.

## Verify browser tab notifications

Build the [web app](../README.md#browser) and use the same disposable
Synapse/Element and Linux desktop prerequisites above. Use a Playwright
version with `connect_over_cdp(no_defaults=True)`. For a loopback-only run:

```sh
bash scripts/rust-web.sh
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 DUNST=/path/to/dunst \
  PYTHON=/path/to/qa/venv/bin/python \
  unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --browser-alerts'
```

The receiver uses Untethered’s Rust/WASM frontend and gomuks worker in a
fresh, headed Chromium profile on Xvfb. The sender is a different verified
user in Element. The fixture clicks **Mute ordinary** and reads that room’s
push rule from Synapse. It then drives **Enable browser notifications** and
the real Chromium **Allow** prompt. No notification constructor, permission
result or page visibility is mocked. A second tab is brought forward;
Playwright’s usual forced-focus emulation is disabled.

The receiving tab must be hidden and unfocused, with the worker reporting
decrypted message bodies and mention targets. Only the direct mention after
permission is granted may produce a popup. OCR reads its body on screen;
Dunst’s history must identify Chromium as the sender. The fixture activates
the popup, checks the tab regains focus, reloads, and sends another ordinary
message in the background. Across all controls there must be exactly one
`org.freedesktop.Notifications.Notify` call and one archived notification.
