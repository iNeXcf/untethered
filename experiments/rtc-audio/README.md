# CALL-01: native encrypted audio probe

This standalone crate tests the media half of a future Matrix call without adding
a WebRTC binary or its build requirements to the main chat client. Two Rust
LiveKit 0.12 peers use separate, ephemeral participant keys, publish 480 Hz and
960 Hz PCM, and check that each remote GCM-encrypted track decodes to the other
peer's frequency. No microphone, speakers, real Matrix account, or call invite.

## Replay the Linux test

Install the build prerequisites for the LiveKit Rust SDK (including Clang, CMake,
and a C++ toolchain for the underlying WebRTC library). Put a `livekit-server`
binary on disk. The verified test used server 1.13.11.

```sh
cd experiments/rtc-audio
cargo build --locked -j2
unshare -Urn sh -ec '
  ip link set lo up
  ip link add rtc-test type dummy
  ip addr add 192.0.2.2/24 dev rtc-test
  ip link set rtc-test up
  RTC_PROBE_ICE_IP=192.0.2.2 python3 run_local.py "$1"
' probe /absolute/path/to/livekit-server
```

The isolated namespace needs an ICE interface other than bare loopback. The
runner starts the test SFU, creates short-lived test tokens for a unique room,
then stops the server. It does not use the homeserver or its credentials.

## Replay the MatrixRTC membership check

With the Python and extracted Element Web prerequisites from the
[Element test setup](../../scripts/kwin-element.md), run from the checkout:

```sh
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow --with cryptography \
    python scripts/test-media-interop.py --rtc-membership
```

This test uses two disposable accounts in an encrypted room and no compositor
or SFU. The Rust client uses its native backend and authenticated gomuks RPCs.
A Chromium instance running Element reads **its own MatrixRTC participant
list**. It checks the native user, device, member ID, room-wide voice slot and
advertised transport. Synapse's eight-second delayed leave is renewed; the
observer verifies the member survives the original deadline and disappears
on explicit leave. A second join ends by disconnecting the native backend:
Element sees the roster empty and Synapse confirms the empty state event.

The probe implements the MSC3401 session membership that this Element release
accepts in numbered room versions. It has a one-minute upper bound, even if the
client dies before arming the delayed leave. A long call will also need to
refresh the membership itself. The advertised transport in this check is a
sentinel, so a roster entry does not demonstrate a media connection.

## What comes next

The main client can already call gomuks `get_rtc_transports` from **Account →
Check call server support**. The next interoperability milestone is to
obtain the LiveKit token for that session through gomuks, and bridge Element's
encrypted per-sender media-key events and key indices to the native
provider (including rotation). The local probe provisions keys directly to test
the media stack; it does not implement those steps. Only claim Matrix calling
after Element ↔ Untethered exchanges encrypted audio both ways. Mic selection,
ringing, mute/deafen, network changes, relay tests and Windows runtime tests
remain part of that milestone.
