# Verify room posting permissions

Use the [native Iced and Element test environment](replies-element.md):

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --room-permissions
```

The fixture creates an encrypted room on loopback Synapse with two disposable
accounts: the room administrator in Untethered and an ordinary member in
Element. It reads Element’s actual `/send/m.room.encrypted` HTTP responses,
checks the room timeline independently, and reads the delivered text on the
native display.

| State | Observation |
|---|---|
| Initially writable | The member sends an encrypted message; HTTP 200 and visible on both clients. |
| **Make read-only** | Both encrypted and plain member sends return HTTP 403 `M_FORBIDDEN`. Neither enters the room history. |
| Still read-only | The member’s power-level write is also forbidden. An administrator’s encrypted announcement reaches Element. |
| **Restore posting** after a native restart | The member receives HTTP 200 for a new send, and Untethered decrypts it. The denied attempts are absent. |

The preset raises `events_default` and any lower explicit levels for
`m.room.message`, `m.room.encrypted`, `m.sticker` and `m.reaction`. The floor
is 50, or one above `users_default` if that is higher. Other explicit event
rules and user roles retain their values. Before writing `m.room.power_levels`,
Untethered re-reads room state and checks that the power-level event ID still
matches the one used to display the control.

`com.untethered.posting_restore.v1` in the power-level content records only
the changed posting fields and their previous values. Restore removes this
record and restores fields still at the preset level. The test compares the
entire resulting document to the baseline, including an originally absent
`events_default`, except for two later changes made by the administrator’s
other session: a custom audit rule and a stricter reaction rule.
