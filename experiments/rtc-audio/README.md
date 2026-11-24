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

## What comes next

The main client can already call gomuks `get_rtc_transports` from **Account →
Check call server support**. The next interoperability milestone is to use
gomuks to join the same MatrixRTC session as Element: agree on room/slot/member
identity, publish/renew membership and delayed leave, obtain the LiveKit token,
and bridge Element's per-sender media-key events and key indices to the native
provider (including rotation). The local probe provisions keys directly to test
the media stack; it does not implement those steps. Only claim Matrix calling
after Element ↔ Untethered exchanges encrypted audio both ways. Mic selection,
ringing, mute/deafen, network changes, relay tests and Windows runtime tests
remain part of that milestone.
