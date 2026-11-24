# CALL-01: native encrypted audio probe

This standalone crate tests the media path alongside the Linux desktop client.
The interoperability tests use **one Rust participant and one
unmodified Element Call widget**. The older two-Rust-peer test remains useful
for diagnosing transport problems independently of Matrix.

## Replay the Linux desktop DM call

Build the actual Iced app with `cargo build --locked -j2 --manifest-path
rust-client/Cargo.toml`, then run `sh experiments/rtc-audio/run_element.sh
--desktop-dm` with the environment variables shown below. In addition to
the common requirements, install Xvfb, xdpyinfo, xdotool, ImageMagick,
Tesseract, and PulseAudio (`pulseaudio`, `pacat`, and `pactl`). The test starts
a private X display and a private audio server with two null sinks. It clicks
**Voice call** in a live encrypted DM in Untethered, waits for Element's
incoming-call toast and ringtone, then accepts. It checks the 480 Hz desktop
mic at Element and the 960 Hz Element mic at the desktop speaker monitor,
checks both GCM publications at the SFU, waits past the first 60-second
membership expiry, confirms renewed membership and continuing audio, then
clicks the desktop **Hang up**.
It requires an empty remote call roster and no remaining desktop audio
streams. Neither test client needs physical audio hardware.

For the reverse direction, use `sh experiments/rtc-audio/run_element.sh
--incoming-dm` with the same setup. Element originates two distinct calls.
On the first, the desktop speaker monitor must detect the 440/620 Hz ringtone;
**Decline** must reach Element as an encrypted event referencing that exact
invite. A PulseAudio event subscription checks that no microphone stream
was created, even transiently. On the second, Iced **Accept** must open one
microphone; both distinct tones and both GCM publications are checked again.
Hanging up from Element must empty its roster and release both desktop streams.

### Verify three-person voice rooms

Build the Linux desktop and use the same desktop test requirements above:

```sh
LIVEKIT_SERVER=/path/to/livekit-server LK_JWT_SERVICE=/path/to/lk-jwt-service \
  PYTHON=/path/to/qa/venv/bin/python sh experiments/rtc-audio/run_element.sh --voice-room
```

This test starts three Untethered processes with different verified accounts,
separate displays and dedicated virtual microphones/speaker monitors. It
clicks **Join voice** in the encrypted *Atrium* call room. The first two must
exchange audio before the third joins. With all three present, every speaker
must contain the other two frequencies and exclude its own for twelve
consecutive 100 ms windows. The test checks two local playback streams per
client, three GCM microphone publications, the names under the channel, and
the room history (no non-state timeline messages, including encrypted rings).
With all three mics still active, the test opens Alice’s **Mixer** and drives
the rendered sliders. It measures each sender’s frequency at every receiver’s
speaker monitor for twelve fresh 100 ms windows per setting. At 25%, only
Bruno’s tone at Alice should drop to one quarter of baseline. Boosting Clara
to 150% on Alice must leave Bruno there at 25% and both other listeners
unchanged. Setting Bruno to zero must silence only him for Alice; **Reset**
must restore the baseline mix. The three GCM microphone track IDs must survive.

It then feeds one microphone at a time, with silence between speakers. On
each of the three rendered sidebars, the **Speaking** label must line up with
the correct name and disappear during silence, while the participants and
microphone track IDs stay the same. An overlapping pair must show two labels;
when only one falls silent the other label must remain. Muting the remaining
speaker while her test input is still loud must clear the last label, and
unmuting must restore it. Each step checks the tones received by all three clients.
After one leaves, the remaining two must exchange newly injected tones on
their original participant and track IDs. Final leave must clear call
memberships and release all capture/playback streams.

### Verify portal window sharing

Use the three-client setup above, plus `pipewire`, `wireplumber`, `pw-dump`,
`busctl`, GStreamer's PipeWire and base video plugins, PySide6 for the two
test windows (`/usr/bin/python`), and the [isolated KWin prerequisites](../../scripts/kwin-element.md).
Run `sh experiments/rtc-audio/run_element.sh --window-share` with
`KWIN_INPUTSYNTH` pointing to the installed KDE test input helper. No Element
Web archive is needed. Only the sharer runs on KWin; the receivers each have
a private X display. The virtual compositor, PipeWire server and session bus
are local to the fixture.

The test cancels a real KDE picker, then selects **Prism** from a fresh picker
with both Prism and a decoy window visible. Both receiver screenshots must
contain the selected window's adjacent pink/teal fields at video-tile scale.
After the sharer navigates to a text channel, the test changes the source
to blue/orange and injects new mic tones. It checks the new pixels at both
receivers, all six decoded audio paths, the original microphone IDs and a GCM
`SCREEN_SHARE` publication. **Stop sharing** must remove the portal session,
restore the PipeWire video graph to baseline, unpublish the video and clear
both receiver tiles while the voice room stays connected.

### Verify voice-room camera video

Use `sh experiments/rtc-audio/run_element.sh --room-camera` with the same
three-client and [private KWin setup](../../scripts/kwin-element.md). The fixture
provides a virtual camera using GStreamer's `rawvideoparse` and `pipewiresink`
on its own PipeWire server; it does not open a physical webcam. The sender can
reach the camera only through the real Camera portal's file descriptor.

The test starts with no camera, then cancels a pending permission request and
checks that no capture process, PipeWire link or SFU video track exists. On a
fresh grant it verifies an active link to the virtual camera and a GCM
`CAMERA` publication. Screenshots of both peers must show the green/magenta
test fields, then fresh orange/blue fields on the same video track. **Stop
camera** must remove the capture process, camera links, SFU video track and
both remote tiles while leaving the virtual camera available. Changing mic
tones verifies all six audio paths both during video and after stopping it.

### Verify mute and deafen

Use `sh experiments/rtc-audio/run_element.sh --call-controls` with the same
setup. The test clicks **Mute**, switches to another room, and uses the active
DM's call bar there. It tests deafen/undeafen both with and without a prior
manual mute. Each transition requires 12 fresh 100 ms samples at **both**
ends: a through-tone or silence (Element FFT peak below −65 dB; desktop peak
below 32 in signed 16-bit PCM). It also checks the actual capture-stream
count and the SFU mute flags. Throughout the test the browser receiver, track,
SSRC and signaling sockets stay the same, and both participants keep their
original GCM publications on the SFU.

### Verify microphone and speaker selection

Run `sh experiments/rtc-audio/run_element.sh --call-devices` with the same
setup. Two virtual microphones continuously supply **480 Hz** and **730 Hz**,
and both virtual speaker outputs are monitored for Element's **960 Hz**.
The test clicks the Iced **Devices** selectors. After each switch it requires
12 fresh 100 ms windows at Element and at each speaker: the chosen mic tone,
the return tone at the chosen output, and silence at the other. It checks
PulseAudio stream routes, unchanged server defaults, the original two GCM
publication IDs and participant IDs, and browser media/signaling continuity.

It also selects a spare mic while muted and removes it before unmuting.
There must be no fallback capture: Element stays silent, playback continues,
and explicitly choosing a working mic restores audio on the same track.

### Verify recovery after a network interruption

For the desktop network recovery test, run
`sh experiments/rtc-audio/run_element.sh --call-reconnect` with the desktop
prerequisites, plus `nsenter`, `ss` with socket-destroy support and `nft`.
The runner gives Untethered and its gomuks child their own network namespace
and veth; Element, Synapse and LiveKit stay online in the outer test namespace.
It takes the link down for at least 6.5 s and discards the desktop's established
SFU signaling socket. The test observes **Reconnecting**, no arriving RTP,
silence and closed desktop audio devices. It changes the test mic from 480 Hz
to 730 Hz while disconnected. After link-up it requires 12 fresh samples of
the new tone at Element and 12 of the 960 Hz return tone at the desktop, both
GCM tracks, unchanged MatrixRTC membership and exactly one ring event. These
are short-outage checks; they do not exercise TURN or resume an expired call.

### Verify TURN-only calls

With the debug Linux desktop build and desktop test environment above, install
`coturn`, `nftables`, iproute2 and `nsenter`. Run the GUI tests one at a time:

```sh
sh experiments/rtc-audio/run_element.sh --call-turn
sh experiments/rtc-audio/run_element.sh --call-turn --incoming-dm
```

The real desktop/gomuks and Chromium/Element clients run on separate private
subnets. Before dialing, each client gets default-deny IPv4/IPv6 firewall
rules: only local IPC, the test services' TCP signaling ports and authenticated
UDP TURN at `192.0.2.3:3478` are reachable. LiveKit advertises a disposable
coturn 4.19.2 instance; each firewall counts drops to the SFU's direct media
ports. The runner does not force either client's ICE policy.

At both clients, active inbound/outbound RTP stats are traced through their
transport's `selectedCandidatePairId` to the **local relay candidate**. The
Rust debug build's opt-in `UNTETHERED_QA_ICE_STATS=1` diagnostic reports only
direction, candidate type, relay protocol and packet count. The assertion
requires `relay/udp` in all four cases, with live traffic at both firewalls.
Element must decode 480 Hz and the Linux speaker must render 960 Hz. Blocking
only TURN on the desktop must silence **both** while the call stays joined.
After unblocking, the runner verifies a newly injected 730 Hz at Element and
960 Hz at the desktop for 12 fresh 100 ms windows, with GCM on both microphone
publications. This covers TURN/UDP, not TURN/TCP or TURN/TLS.

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
client dies before arming the delayed leave. The desktop renews the lease on
longer calls. The advertised transport in this membership-only check is a
sentinel, so a roster entry does not demonstrate a media connection.

## What comes next

Element and Untethered can each initiate DM calls while both apps are open.
The isolated relay test covers both caller directions. Physical hot-unplug
behavior and Windows runtime testing are not covered by the virtual-device tests.
