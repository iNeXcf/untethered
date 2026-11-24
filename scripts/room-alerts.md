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
