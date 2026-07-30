# Auto-Mute on Walkaway

A privacy-first desktop app that notices when you step away from your desk
during a meeting and automatically **mutes your microphone** and **disables
your camera**, restoring both when you return.

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
| Meeting       | Core (logic)   | ✅ Done      |
| Orchestration | Application    | ✅ Done      |
| Microphone    | Infrastructure | ✅ Done (PulseAudio / PipeWire via `pactl`) |
| Clock         | Infrastructure | ✅ Done      |
| Notification  | Infrastructure | ✅ Done (Tauri notification plugin) |
| Tray          | Infrastructure | ✅ Done      |
| Settings UI   | UI             | ✅ Done (React) |
| Tauri wiring  | UI / OS        | ✅ Done      |
| Camera        | Infrastructure | ⚠️ Platform-limited (see below) |
| Presence capture (MediaPipe) | Infrastructure | ⏳ Next |
| Meeting detection | Infrastructure | ⏳ Next  |

The **entire walkaway decision logic — presence debouncing, meeting gating,
auto-mute, auto-camera-off, and auto-restore — is implemented and unit-tested**,
and it is now wired end-to-end into a Tauri desktop app: a background supervisor
owns the controller, the microphone is really muted through `pactl`, actions
raise desktop notifications, a tray gives show/quit, and a React Settings panel
edits the persisted config live.

**What still feeds the logic manually.** The presence detector (webcam +
MediaPipe) and the meeting detector are the remaining modules. Until they land,
the Settings window exposes two manual inputs — *"In a meeting"* and *"Away"* —
that drive the **real** protection path (they mute your actual mic). Those
detectors will push into the exact same sampling channel, so no application code
changes when they arrive.

**Camera is platform-limited by design, not stubbed.** Unlike audio, Linux has
no portable way to force another app's camera off — the capture device is owned
by the conferencing app. The `UnsupportedCamera` adapter therefore reports the
capability as unavailable, and the controller (which already tolerates a failing
device port) simply leaves the camera alone while still muting the mic. A future
per-platform backend can replace that adapter without touching the core.

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
  domain/        Core: presence & meeting state machines, events, value objects
  config/        Infrastructure: typed JSON config with defaults & validation
  logger/        Infrastructure: leveled logger with pluggable sinks
  eventbus/      Application: synchronous in-process pub/sub
  application/   Use cases: WalkawayController + ports (device interfaces)
  adapters/      Infrastructure: OS adapters implementing the ports
                 (SystemClock, PulseMicrophone, UnsupportedCamera)
src-tauri/       UI/OS: the Tauri app that composes the above and hosts the UI
src/             UI: the React + TypeScript Settings front end
```

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

1. A presence sample (`face_present: bool`) arrives each tick. `PresenceTracker`
   debounces it — a face must be *continuously* absent for a grace period before
   the user is declared `Away`, and continuously present before `Present`. This
   rejects single dropped frames.
2. A meeting sample (`active: bool`) updates `MeetingTracker`.
3. `WalkawayController` reconciles: it protects devices only while a meeting is
   **active and** the user is **away**. On engage it mutes the mic / disables the
   camera, remembering the prior state — but only for devices it actually
   changed, so a user's own manual mute is never disturbed.
4. On return, meeting end, or any exit from the protected condition, it restores
   exactly what it changed (when `auto_restore` is enabled).

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

The logic crates require only a Rust toolchain (1.75+):

```bash
cargo test --workspace          # run all unit tests
cargo clippy --workspace --all-targets -- -D warnings
```

### Running the desktop app

The Tauri shell additionally needs Node.js and the platform webview
dependencies (on Linux: `webkit2gtk-4.1`, `gtk3`, `libsoup-3` — see the Tauri
v2 prerequisites for your OS), plus `pactl` for microphone control.

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
