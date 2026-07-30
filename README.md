# Auto-Mute on Walkaway

A privacy-first desktop app that notices when you step away from your desk and
automatically **mutes your microphone** and **disables your camera**, restoring
both when you return. You switch protection on, and it watches for you.

Everything runs locally. No camera frames, audio, images, or user data ever
leave the machine.

---

## Status

This repository is being built module by module, foundation first. Each module
is completed to production quality — with tests — before the next is started.

| Module        | Layer          | State        |
| ------------- | -------------- | ------------ |
| Config        | Infrastructure | ✅ Done      |
| Logger        | Infrastructure | ✅ Done      |
| Event Bus     | Application    | ✅ Done      |
| Presence      | Core (logic)   | ✅ Done      |
| Orchestration | Application    | ✅ Done      |
| Microphone    | Infrastructure | ✅ Done (Windows WASAPI · Linux PulseAudio/PipeWire) |
| Clock         | Infrastructure | ✅ Done      |
| Notification  | Infrastructure | ✅ Done (Tauri notification plugin) |
| Tray          | Infrastructure | ✅ Done      |
| Settings UI   | UI             | ✅ Done (React) |
| Tauri wiring  | UI / OS        | ✅ Done      |
| Camera        | Infrastructure | ✅ Done (Windows SetupAPI · Linux `uvcvideo` — see below) |
| Presence capture (MediaPipe) | Infrastructure | ✅ Done (Python sidecar, see `presence-detector/`) |
| Host bridge (sidecar → Event Bus) | UI / OS | ✅ Done (spawns the sidecar, feeds presence in) |

The **entire walkaway decision logic — presence debouncing, the protection
master switch, auto-mute, auto-camera-off, and auto-restore — is implemented and
unit-tested**, and it is now wired end-to-end into a Tauri desktop app: a
background supervisor owns the controller, the microphone is really muted through
`pactl`, actions raise desktop notifications, a tray gives show/quit, and a React
Settings panel edits the persisted config live.

**Presence detection is implemented as a local sidecar, now wired end-to-end.**
The webcam + MediaPipe presence detector lives in
[`presence-detector/`](presence-detector/) as a self-contained Python module
(OpenCV + MediaPipe have no production-grade Rust binding). It watches the
camera on-device and emits `present` / `leaving` / `away` / `returning` events
as newline-delimited JSON on stdout — the Event Bus contract. It holds no
business logic and controls no devices; it only reports presence.

The **host bridge** (`src-tauri/src/bridge.rs`) closes the loop: on startup it
spawns the sidecar, reads its event lines, and translates each into a
face-presence sample fed to the controller — which then drives mute / camera-off
/ restore and announces every action on the `EventBus`. The translation is a
small, tested unit (`amow_application::presence_source`); crucially it maps on
*face visibility*, so the sidecar's transitional `leaving` / `returning` edges
pass straight through and the **single** configurable debounce stays in the core
(`presence.away_grace_ms` / `return_grace_ms`) — never duplicated across the two
processes. Only presence phases cross the process boundary; no camera frame ever
does. If the sidecar cannot start (no Python, no camera), the app logs it and
falls back to the manual *"Away"* toggle. Protection itself is a single master
switch the user turns on: while it is on, a walkaway mutes the mic and disables
the camera; turning it off (or returning) restores them. The manual toggles
drive the **real** protection path (they mute your actual mic).

**Camera control is real, at the OS level, on both platforms.** On Windows the
`WindowsCamera` adapter disables and re-enables the webcam's device node through
SetupAPI / Configuration Manager — the same operation as Device Manager's
"Disable device", which removes the camera from *every* application until it is
re-enabled. On Linux the `LinuxUvcCamera` adapter does the equivalent by binding
/ unbinding the webcam from the `uvcvideo` kernel driver through sysfs; an
unbound device disappears from `/dev/video*`. Both are genuine system-wide
switches (not per-app hints) that work even while the camera is in use — the
walkaway case — and unlike `modprobe -r uvcvideo` they act per-device. Each
adapter remembers exactly which devices it turned off and restores only those on
return, so a camera the user disabled themselves is never re-enabled. Toggling a
device needs elevated privileges; where the app lacks them the calls fail and it
degrades to the safe `UnsupportedCamera` behaviour — the controller logs the
error and leaves the camera alone while still muting the mic.

**One microphone port, native on each OS.** On Windows `WindowsMicrophone` mutes
the default *communications* capture endpoint through the WASAPI Core Audio
`IAudioEndpointVolume` interface; on Linux `PulseMicrophone` drives the default
source through `pactl` (PulseAudio / PipeWire). Both implement the same
`Microphone` port, so the controller is identical across platforms.

**Portable by construction, tested without hardware.** Every OS adapter hides its
platform calls behind a small injected seam — `CommandRunner`, `Sysfs`,
`EndpointVolume`, `CameraDevices` — so all the decision logic (which devices to
toggle, what to restore, how failures propagate) is unit-tested against fakes on
any machine, with no audio server, webcam, or root required. The actual
`windows`-crate syscalls compile only on Windows (a target-gated dependency), and
the composition root selects the host's native pair at compile time via
`PlatformMicrophone` / `PlatformCamera` — no runtime `dyn`, and other OS backends
(e.g. macOS) can be added later without touching the core.

---

## Architecture

Clean Architecture with a strict dependency direction — inner layers never know
about outer ones:

```
UI (React)  →  Application  →  Core (domain)  ←  Infrastructure  →  OS
```

The Rust code is split into a Cargo workspace of **pure-logic crates** plus a
future `src-tauri` application crate:

```
crates/
  domain/        Core: presence state machine, events, value objects
  config/        Infrastructure: typed JSON config with defaults & validation
  logger/        Infrastructure: leveled logger with pluggable sinks
  eventbus/      Application: synchronous in-process pub/sub
  application/   Use cases: WalkawayController + ports (device interfaces)
  adapters/      Infrastructure: OS adapters implementing the ports
                 (SystemClock; WindowsMicrophone/PulseMicrophone;
                  WindowsCamera/LinuxUvcCamera/UnsupportedCamera). The host's
                  native pair is chosen at compile time via PlatformMicrophone /
                  PlatformCamera + default_microphone() / default_camera().
src-tauri/       UI/OS: the Tauri app that composes the above and hosts the UI
src/             UI: the React + TypeScript Settings front end
presence-detector/  Infrastructure: Python webcam presence sidecar
                    (OpenCV + MediaPipe) emitting presence events as JSON
```

The presence sidecar is a separate process in Python because OpenCV/MediaPipe
have no production-grade Rust binding. It communicates over a narrow,
language-neutral contract (JSON lines on stdout), so it stays fully decoupled
from the Rust core — see [`presence-detector/README.md`](presence-detector/README.md).

`src-tauri/` is deliberately **excluded from the Cargo workspace** so
`cargo test` on the logic crates never needs the GUI toolchain (webkit2gtk).

Key design choices that keep this honest and testable:

- **The core has no I/O and no clock.** Time is injected as a monotonic
  millisecond count, so every state transition is deterministic under test.
- **Dependency Inversion for devices.** The application depends on `Microphone`,
  `Camera`, `Notifier`, and `Clock` *traits* (ports). OS adapters implement them;
  the application never names PulseAudio, CoreAudio, or any concrete technology.
- **Event-driven.** The core emits past-tense `DomainEvent`s onto the event bus;
  logging, tray, notifications, and the UI react to those rather than to each
  other.
- **Logic is decoupled from Tauri.** Because business logic lives in plain
  crates, `cargo test` verifies it with no GUI toolkit installed.

### How the walkaway logic works

1. The user turns **protection on** (the master switch). Nothing is ever touched
   while it is off.
2. A presence sample (`face_present: bool`) arrives each tick — from the webcam
   sidecar via the host bridge, or from the manual toggle. `PresenceTracker`
   debounces it — a face must be *continuously* absent for a grace period before
   the user is declared `Away`, and continuously present before `Present`. This
   rejects single dropped frames.
3. `WalkawayController` reconciles: it protects devices only while protection is
   **on and** the user is **away**. On engage it mutes the mic / disables the
   camera, remembering the prior state — but only for devices it actually
   changed, so a user's own manual mute is never disturbed.
4. On return, when protection is switched off, or any exit from the protected
   condition, it restores exactly what it changed (when `auto_restore` is
   enabled).

---

## Configuration

Config is a human-editable JSON document with safe defaults and validation.
Missing fields fall back to defaults, so new settings can be added without
breaking existing files. Example:

```json
{
  "version": 1,
  "behavior": {
    "auto_mute": true,
    "auto_camera_off": true,
    "auto_restore": true,
    "notify_on_action": true,
    "sample_interval_ms": 500
  },
  "presence": {
    "away_grace_ms": 3000,
    "return_grace_ms": 800
  },
  "logging": { "level": "info" }
}
```

---

## Building & testing

The logic crates require only a Rust toolchain (1.75+) and build on every OS —
the native Windows device code is target-gated, so a non-Windows `cargo test`
never pulls in the `windows` crate:

```bash
cargo test --workspace          # run all unit tests (any OS)
cargo clippy --workspace --all-targets -- -D warnings

# Verify the Windows adapters from another OS without a Windows box:
cargo clippy -p amow-adapters --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

### Running the desktop app

The Tauri shell additionally needs Node.js and the platform webview
dependencies. On **Linux**: `webkit2gtk-4.1`, `gtk3`, `libsoup-3` (see the Tauri
v2 prerequisites for your OS), plus `pactl` for microphone control. On
**Windows**: WebView2 (preinstalled on Windows 10/11) and the MSVC build tools;
microphone and camera control use built-in OS APIs, no extra runtime.

Camera control (`auto_camera_off`) is a real OS-level switch that needs elevated
privileges to toggle the device. On Linux it writes
`/sys/bus/usb/drivers/uvcvideo/{bind,unbind}` (grant access with a udev rule or
run privileged); on Windows it enables/disables the camera device node via
SetupAPI (run elevated / as administrator). Without the privilege the app keeps
working: the camera is left untouched and the microphone is still muted.

Presence detection is launched automatically as a sidecar. Install its optional
dependencies (`pip install -e 'presence-detector[camera]'`, which pulls in
`opencv-python` and `mediapipe`) so the webcam detector can run. The bridge
finds the module beside the executable or under `presence-detector/` in the
working directory; override the interpreter, module location, or disable it
entirely with `AMOW_PRESENCE_PYTHON`, `AMOW_PRESENCE_DIR`, and
`AMOW_PRESENCE_DISABLE=1`. If it can't start, the app falls back to the manual
*"Away"* toggle.

```bash
npm install                 # front-end dependencies
npm run tauri dev           # run the app (Vite + Tauri)
npm run tauri build         # produce a release bundle
```

Config is stored at the OS app-config dir (e.g. `~/.config/com.automute.walkaway/config.json`)
and logs at the app-log dir; neither is committed.

---

## Non-functional targets

- **Startup** < 2 s · **Idle CPU** < 3 % · **RAM** < 150 MB · **Offline-first**.
- The event bus is synchronous and lock-guarded (no async runtime) precisely to
  keep startup fast and idle cost near zero for the app's tiny event volume.

## Privacy

The app never uploads camera frames, audio, images, or user data. All detection
runs on-device; the logger records only short status text, never media or
personal data.

## License

MIT
