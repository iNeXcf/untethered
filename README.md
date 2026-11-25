# untethered 

An independent Matrix desktop prototype with a Qt Quick interface and a local gomuks backend. It opens as a normal Linux app and stores the Matrix session under the current user's local application data directory.

## Build

Run `makepkg -s` in this directory. The recipe downloads the official x86-64 gomuks desktop release, checks its SHA-256 hash and packages its gomuks backend alongside the Qt application and the pinned LiveKit C++ SDK. Element Call and Qt WebEngine are no longer used. For a development build, run `bash scripts/dev.sh run`. The script downloads and verifies the same SDK, or uses the unpacked SDK named by `LIVEKIT_SDK`.

The backend is [gomuks v0.2611.1](https://github.com/gomuks/gomuks/releases/tag/v0.2611.1), with [source at commit 5d21e2c](https://github.com/gomuks/gomuks/tree/5d21e2c7b31aabbba30efdd33f5882d99972e00e). Qt comes from system packages. The package is for local prototype testing.


## Local startup test

Run `untethered --smoke-test` to test Qt startup and the backend handshake. Set `XDG_DATA_HOME` to a fresh directory for an isolated profile. Developers can pass `--backend /absolute/path/to/gomuks` when running an unpackaged build.

## Calls

Open a Matrix room and select **Call**. Calls use the homeserver’s advertised MatrixRTC service and start with camera and microphone off. The microphone, camera, screen sharing and audio devices are controlled in untethered. **Back to chat** keeps the call connected, while **Leave call** disconnects and stops local capture. Wayland screen sharing requires an xdg-desktop-portal backend with screen capture support and PipeWire.

The native call client currently connects to one LiveKit server per call. All participants need a compatible transport and must be reachable on that server.
