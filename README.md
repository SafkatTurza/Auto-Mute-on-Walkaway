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
| Microphone    | Infrastructure | ⏳ Next      |
| Camera        | Infrastructure | ⏳ Next      |
| Presence capture (MediaPipe) | Infrastructure | ⏳ Next |
| Notification  | Infrastructure | ⏳ Planned   |
| Tray          | Infrastructure | ⏳ Planned   |
| Settings UI   | UI             | ⏳ Planned   |
| Tauri wiring  | UI / OS        | ⏳ Planned   |

The **entire walkaway decision logic — presence debouncing, meeting gating,
auto-mute, auto-camera-off, and auto-restore — is implemented and unit-tested**
today. What remains is the OS-specific plumbing (device adapters, the webcam
capture pipeline, tray, and the Tauri/React shell) that connects that logic to
the machine.

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
```

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

The Tauri desktop shell (once added) additionally needs the platform webview
dependencies — see the Tauri prerequisites for your OS.

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
