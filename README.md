# untethered 

An independent Matrix desktop prototype with a Qt Quick interface and a local gomuks backend. It opens as a normal Linux app and stores the Matrix session under the current user's local application data directory.

## Build

Run `makepkg -s` in this directory. The recipe downloads the official x86-64 gomuks desktop release, checks its SHA-256 hash and packages only its gomuks backend alongside the Qt application.

The backend is [gomuks v0.2611.1](https://github.com/gomuks/gomuks/releases/tag/v0.2611.1), with [source at commit 5d21e2c](https://github.com/gomuks/gomuks/tree/5d21e2c7b31aabbba30efdd33f5882d99972e00e). Qt comes from system packages. The package is for local prototype testing.

## Develop in Code OSS

Open this project folder in Code OSS. Use **Terminal → Run Task → untethered: Run**
to build and launch the app. **Ctrl+Shift+B** builds it without launching.
The first run downloads and verifies the pinned gomuks backend. Later runs reuse it
and build the frontend incrementally. No package installation is required.

From a shell the equivalent is `bash scripts/dev.sh run`. The development
profile lives under `.dev/profile/untethered` and has a separate Matrix session.
The installed package and its profile can be used independently.

To uninstall the package, run `sudo pacman -R untethered`. Pacman removes
the installed app and backend but keeps your Matrix session and encryption keys
under `~/.local/share/untethered` (or your configured `XDG_DATA_HOME`).


## Local startup test

Run `untethered --smoke-test` to test Qt startup and the backend handshake. Set `XDG_DATA_HOME` to a fresh directory for an isolated profile. Developers can pass `--backend /absolute/path/to/gomuks` when running an unpackaged build.
