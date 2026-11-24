# CALL-01: native encrypted audio probe

This standalone crate tests the media path before adding WebRTC to the chat
client. The Linux interoperability test uses **one Rust participant and one
unmodified Element Call widget**. The older two-Rust-peer test remains useful
for diagnosing transport problems independently of Matrix.

## Replay the Element ↔ Rust encrypted audio test

From the checkout, with Chromium, OpenSSL, iproute2, the pinned gomuks backend,
and the Python dependencies used by the membership check below:

```sh
cargo build --locked -j2 --manifest-path experiments/rtc-audio/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  LIVEKIT_SERVER=/path/to/livekit-server \
  LK_JWT_SERVICE=/path/to/lk-jwt-service_linux_amd64 \
  PYTHON=/path/to/qa/venv/bin/python \
  sh experiments/rtc-audio/run_element.sh
```

Verified with LiveKit 1.13.11 and `element-hq/lk-jwt-service` 0.10.0. The
runner starts a private network namespace, Synapse, the SFU, its authorization
service, and two disposable accounts. Element's **Voice call** button opens
the bundled widget with a 960 Hz WAV as its test microphone. On Linux the
Rust peer joins the same MatrixRTC slot, requests an OpenID token from gomuks
and exchanges it at the advertised `/sfu/get` endpoint. The authorization
service verifies the token against Synapse over TLS and issues the SFU JWT.
Neither peer is provisioned with the other's media key.

Both peers publish GCM-encrypted audio using per-participant keys and HKDF.
The Rust peer accepts Element's key only from an encrypted to-device event
matching the room, sender and participating device/member. The test requires:

- Rust decodes Element's **960 Hz** tone across 20 consecutive PCM frames.
- With Rust's key withheld, the widget receives at least **60 RTP packets**,
  with no sustained **480 Hz** tone in its inbound audio.
- After gomuks sends the key to that Element device, the widget measures
  **480 Hz** for ten consecutive spectra of its received WebRTC track.
- During that same call, Rust installs a fresh sender key at index **1**,
  selects it on the existing frame cryptor, and submits **730 Hz** on the same
  microphone publication. With key 1 withheld, the test requires at least
  **80 additional RTP packets** and **18 additional spectra** with no 730 Hz
  detection. After gomuks delivers key 1 over Olm, Element must detect the new
  tone for ten consecutive spectra. The browser checks the same receiver,
  track, SSRC and open SFU signaling socket throughout; Rust checks the same
  GCM publication and no reconnect or republish events.

This is a synthetic microphone on a local ICE network. It does not exercise
real capture/playback hardware, TURN, incoming DM ringing, or a rotation
initiated by Element.

## Replay the two-Rust-peer test

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

Move the proven signaling and media path into the desktop client. Keep the
active call independent of navigation; handle incoming key changes, microphone
and output devices, mute/deafen, DM ringing and hangup. Verify relay-required
audio separately. Windows runtime testing is deferred.
