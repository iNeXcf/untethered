# Untethered: roadmap to Discord parity

**2026-11-24 · `rust-discord-ui` at `bfa75b3`**

**Next: drag-and-drop attachments and voice calls.** Start the technical proof for both now. Ship files first while proving encrypted call interoperability with Element. This is a delivery order, not a calendar estimate.

## First implementation checkpoint

- **Files available:** Native and web staging tray, multi-file drop/picker, explicit Send,
  per-room drafts, optional caption, sequential encrypted uploads and attachment save.
  Verified a real Dolphin drag between native Wayland windows in an isolated compositor.
  The native live test uploads and saves a byte-identical binary; a separate Chromium
  device also decrypted that file. Element Web 1.12.35 displayed the encrypted attachment
  sent by the browser client. Windows cross-builds; Explorer runtime testing is still open.
- **Composer previews:** local image thumbnails and compact file cards on native and web.
- **Received images:** verified inline encrypted thumbnails on native and Chromium;
  an independent browser device rendered the expected image after both login and reload.
  `scripts/test-media-interop.py` reruns the check with disposable accounts.
- **Native screenshot paste:** verified real Ctrl+V on Wayland and X11 with
  `scripts/test-media-interop.py --native-paste {wayland,x11}`. Each had zero
  uploads/events while staged and one upload/encrypted image event after Send;
  downloaded ciphertext decrypted to the exact test pixels and the pasted text caption.
- **Progress, Cancel, Retry:** desktop consumes gomuks's progress stream; the
  browser worker uses actual XHR upload events. `scripts/test-media-interop.py
  --upload-cancel` throttles encrypted 16 MiB files: both transports closed
  mid-body with zero new room events; each retry delivered exactly one.
- **Portal-only Wayland drop:** `scripts/test-media-interop.py --portal-drop` uses
  a GTK sender with no URI list and a real private `xdg-document-portal`. Switching
  rooms during retrieval and upload left the other room untouched. The original
  encrypted room received exactly one PNG and one PDF; both decrypted byte-for-byte.
- **Files still to do:** finish the real KDE/KWin and Windows Explorer runtime
  checks and the bidirectional Element image/PDF acceptance matrix below.
- **Calls started:** a user-triggered MatrixRTC transport check in Account, and a
  [repeatable native audio probe](experiments/rtc-audio/README.md). Two Rust peers
  decoded each other's encrypted audio through a test SFU. Element interop, real
  devices, Matrix media-key distribution, ringing and call controls are not done.

## What parity means

A small community should be able to move its everyday conversations here: find a server and its channels, talk in DMs, drop a screenshot, join friends in voice, share a screen, catch up on mentions, and moderate its space. Test those workflows side by side with Discord. A button that looks right but doesn't work with another Matrix client isn't parity.

Keep the frontend in **Rust/Iced** and **gomuks** as the Matrix engine. Share room, attachment and call state between desktop and web; isolate file access, capture, playback and notifications behind platform adapters. Keep native controls for desktop calls. Launching a hosted Element Call page is useful for testing but isn't the finished native calling experience.

**Baseline before this milestone (`bfa75b3`):** server rail and space/DM navigation, grouped plain-text timeline, in-memory room drafts, password login, recovery-key verification, and encrypted messages. Linux and Chromium had exchanged encrypted messages and restored sessions. Windows cross-built but had no runtime acceptance test. Media cards, transfers, calls, and voice channels were not yet implemented.

## A. Drop a file and send it

**Experience:** Drag one or more files from Dolphin or Explorer. Highlight the target conversation by name. Show removable attachment chips, image previews, file sizes and a caption field above the composer. Send only when the user presses Send. The `+` picker and pasted screenshots use the same tray. Drafts and in-flight files stay with their original room when the user switches channels.

1. **Prove native Wayland drop first.** Pinned Iced 0.14 explicitly marks `FileDropped` and `FileHovered` unimplemented on Wayland. Try an upstream fix or a small Rust integration with the existing window and Wayland data device. Verify multiple items, spaces in filenames, `text/uri-list`, and portal-backed transfers. An XWayland window or a file picker does not pass the Dolphin → native Wayland test. Use Iced window events on Windows; use DOM `DataTransfer.files` and `File` objects on web.
2. **One transfer model:** staged → uploading → sending event → confirmed, with cancel/failure/retry per file. Bind each transfer to the room ID when staged, respect the homeserver limit, and show progress without blocking navigation. If upload succeeds but event sending fails, retry with the existing media descriptor. Send one standard Matrix media event per file initially so Element can read it.
3. **Use the transport gomuks actually supports.** Desktop: stream file bytes to authenticated loopback `POST /_gomuks/upload`, set encryption for encrypted rooms, and pass the returned descriptor as `base_content` to `send_message`. The file-path `upload_media` RPC is FFI-only. Browser: add upstream's `wasm-upload` binary transfer to our JSON-only worker bridge. Bound file size and concurrency because the current worker buffers a whole file. Use indeterminate progress when byte progress isn't available.
4. **Receive as well as send.** Keep media descriptors in the timeline model. Render image thumbnails and download cards with filename, type and size. Handle encrypted thumbnails and downloads through gomuks. Wire the upstream media service worker or an equivalent browser download path; our current browser bootstrap doesn't register it.

**Done when:** Drop an image and a PDF into an encrypted room on KDE Wayland, switch channels mid-upload, and receive both in the intended Element room. Open the image, save a byte-identical PDF, then repeat in reverse. Exercise Windows and Chromium, an unencrypted room, an oversized file, cancel, and retry without duplicate sends.

## B. Ring a friend and talk

**Experience:** Start an audio call from a DM; the other party can accept or decline. Show ringing, connecting, and connected states. Keep a compact call bar visible while browsing channels, with mute, deafen, device selection and disconnect. The first release covers one-to-one voice while both apps are open.

1. **Prove two-way encrypted audio before polishing the call UI.** Use two accounts and a test homeserver with MatrixRTC discovery, an SFU and relay connectivity. Evaluate a maintained Rust-facing WebRTC/LiveKit media stack for desktop. Use gomuks for transport discovery, authorization and Matrix signaling where supported. Test native capture/output, negotiated audio, E2EE and key rotation against Element. A token or a successful room join is not evidence that the other person can hear you.
2. **Establish the engine boundary.** gomuks exposes RTC transport discovery, LiveKit token requests, delayed-event operations and encrypted to-device messaging. Its own web frontend uses an Element Call widget. Inventory membership, ringing, call keys, track encryption and cleanup required by the native adapter. Prefer reuse over maintaining a parallel Matrix client. For web, evaluate upstream Element Call behind the same call-state model. Surface missing server infrastructure and unsupported legacy calling explicitly.
3. **Finish the lifecycle:** incoming/outgoing, accept, decline, cancel, no answer, hang up, reconnect and answered-on-another-device. Handle permission denial and device removal. Release the microphone on hangup and expire stale call membership after a crash. No silent downgrade from encrypted media.

**Done when:** Untethered and Element can initiate calls to one another on separate networks, including a relay-required connection. Two Untethered users can do the same. Both hear audio, can change channels without dropping the call, mute/deafen correctly, recover from a disconnect and hang up cleanly. Check a server without RTC support. Record real Linux, Windows and browser results separately; keep the feature experimental if encrypted interop fails.

## Remaining delivery order

| Milestone | Deliverable | Acceptance example |
|---|---|---|
| **3. Voice channels, video and screen sharing** | Join a named voice room without ringing everyone. Show participants under the channel, speaking indicators, per-user volume, push-to-talk. Then add camera, grid/focus modes, and screen/window sharing. Use the desktop portal + PipeWire on Wayland, Windows capture, browser share picker. Track system-audio capture and global hotkeys by platform. | Three people join voice, one shares a window, and another browses text channels without losing audio. Leaving stops capture. |
| **4. Everyday messaging** | Replies and jump-to-original, edits, redactions, reactions, mentions/autocomplete, typing, read markers/unread divider, Markdown/code, real avatars, spoilers, link previews, and drafts that survive restart. | Two clients see the same edited reply and reactions after reconnect, even if the original was outside loaded history. |
| **5. Servers, navigation and discovery** | Ordered/collapsible channel categories, remembered room and scroll position, DM/group chat creation, real member lists, unread/mention badges, Ctrl+K, pins, threads, room links, message search and mentions inbox. Expose how much encrypted history is indexed locally. | Find a week-old reply, jump to context, then return to the prior scroll position. |
| **6. Notifications and presence** | Desktop/browser notifications, per-server and per-channel overrides, DND, presence/custom status, sounds, unread sync. Separate foreground ringing from closed-app ringing; tray/background operation is opt-in. | Answer on one device: the other stops ringing. Muted rooms stay quiet; a direct mention follows the selected override. |
| **7. Community management** | Create/join/leave spaces and rooms, invites, channel ordering, profiles/nicknames, permissions, bans/kicks, reports and moderation history where available. Map Discord-like controls honestly onto Matrix power levels and room permissions. | A moderator sets up a community and a second account actually encounters the configured access limits. |
| **8. Account and release readiness** | SSO/OIDC, logout, session/device management, backup health, recovery, and emoji/SAS verification when supported by the engine. Accessible names, screen-reader and keyboard operation, selectable text, scaling, signed Windows distribution, browser compatibility and update/migration tests. | A new user installs, signs in, recovers history, shares a file, calls someone and upgrades without the terminal or losing device keys. |
| **Later, by demand** | Custom emoji/stickers, voice notes, polls, forum channels, stages, scheduled events, bots/commands and richer moderation integrations. | Prioritize by what real communities miss. Discord connectivity, billing, Nitro and its marketplace are separate products. |

**Across every milestone:** Compare keyboard focus, right-click menus, hover states, scroll anchoring, spacing and empty/error states with Discord. Start accessibility and real Windows testing with the first feature. Keep the emoji-heavy channel-switch regression fixture; measure responsiveness and memory during transfers and calls. Don't rebuild the full timeline for every upload tick. No file contents, tokens or call keys in diagnostics.

## First implementation tickets

1. `FILES-01` — Dolphin → Wayland and Explorer → Windows drop, staged without sending.
2. `CALL-01` — Encrypted native audio with Element; record server requirements, media-stack choice and signaling gaps.
3. `FILES-02` — Encrypted desktop send/receive/save with a byte-for-byte integrity check.
4. `FILES-03` — Multi-file drafts, paste, progress, cancel/retry, browser binary upload and authenticated download.
5. `CALL-02` — DM ringing, persistent call controls and device switching on the working media path.

**Implementation references:** branch `rust-discord-ui`, `rust-client/src/{main,rooms,view}.rs` and `rust-client/src/backend/{native,web}.rs`; [Iced window events](https://docs.rs/iced_core/0.14.0/iced_core/window/enum.Event.html); pinned gomuks [RPC contracts](https://github.com/gomuks/gomuks/blob/v0.2611.1/pkg/hicli/jsoncmd/commands.go), [media handler](https://github.com/gomuks/gomuks/blob/v0.2611.1/pkg/gomuks/mediaupload.go), [WASM upload](https://github.com/gomuks/gomuks/blob/v0.2611.1/web/src/api/wasmclient.ts), [Element Call integration](https://github.com/gomuks/gomuks/blob/v0.2611.1/web/src/ui/widget/ElementCall.tsx), and [widget driver](https://github.com/gomuks/gomuks/blob/v0.2611.1/web/src/ui/widget/widgetDriver.ts).
