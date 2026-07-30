# Auto-Mute on Walkaway — Windows Beta

This guide covers installing and running the first **Windows beta**: building the
installer, installing it, first-run setup, and troubleshooting. The beta ships
the native Windows adapters — microphone mute via WASAPI and camera disable via
SetupAPI / Device Manager.

> **Privacy:** everything runs on your machine. No camera frames, audio, images,
> or user data ever leave the device.

---

## 1. What's in the beta

- **Auto-mute** your microphone and **disable** your camera when you step away,
  restored when you return — gated by a single **Protection** master switch.
- **Native Windows device control** (WASAPI mic + SetupAPI camera).
- **System tray** app: left-click the tray icon to show the window, right-click
  for a Show / Quit menu.
- **Start on login** (opt-in toggle in Settings).
- **Crash logging** to a file, so a windowed build still leaves a diagnosable
  trace if something goes wrong.
- **Settings persistence** to a human-editable JSON file.

Presence detection uses the optional Python webcam sidecar. If it isn't present,
the app falls back to a manual **At desk / Away** toggle, so it is fully usable
out of the box.

---

## 2. Requirements

**To run the app**

- Windows 10 or 11 (64-bit).
- **WebView2 runtime** — preinstalled on current Windows 10/11. If missing, the
  installer offers to fetch it, or get the Evergreen runtime from Microsoft.

**To build the installer from source** (additionally)

- [Rust](https://rustup.rs/) 1.75+ with the **MSVC** toolchain
  (`x86_64-pc-windows-msvc`) and the Visual Studio C++ Build Tools.
- [Node.js](https://nodejs.org/) 18+ and npm.

**Optional — webcam presence detection**

- Python 3.9+ with the detector's extras:
  `pip install -e "presence-detector[camera]"` (pulls in `opencv-python` and
  `mediapipe`). Without it, use the manual Away toggle.

---

## 3. Build the installer

From the repository root:

```powershell
npm install
npm run tauri build
```

The bundler produces a Windows installer under:

```
src-tauri\target\release\bundle\nsis\Auto-Mute on Walkaway_0.1.0_x64-setup.exe
```

(An MSI is also produced under `bundle\msi\` when WiX is available.) The NSIS
installer is configured for **per-user** install, so it needs **no
administrator rights** to install.

The release profile is size- and startup-optimized (`opt-level = "z"`, LTO,
symbols stripped). Dependency versions are pinned in `src-tauri/Cargo.lock` for
a reproducible build.

---

## 4. Install & first run

1. Run the `*-setup.exe`. It installs to your user profile and adds Start-menu
   and (optionally) desktop shortcuts.
2. Launch **Auto-Mute on Walkaway**. The window opens and a tray icon appears.
3. Click **Protection off** to turn it **on** — this is the master switch.
   Nothing is ever muted or disabled while it is off.
4. Presence:
   - If the Python sidecar is installed, presence comes from the webcam
     automatically.
   - Otherwise use **Simulate presence → Away** to trigger a walkaway. This
     drives the *real* logic (it mutes your actual mic).
5. Adjust **Automatic actions** (mute mic / disable camera / restore / notify),
   **Timing** (sample interval and away/return grace), and **Logging**, then
   **Save settings**.

### Start on login

Settings → **Startup → Start automatically on login**. This registers the app
with Windows (per-user registry `Run` key); the preference persists across
restarts because Windows is the source of truth. Toggle it off to unregister.

### Tray

- **Left-click** the tray icon → restore the window.
- **Right-click** → **Show Window** / **Quit**.

---

## 5. Camera control needs elevation

Disabling a camera device toggles its device node (the same action as Device
Manager's *Disable device*), which requires **administrator privileges**. To use
`auto_camera_off`, run the app **as administrator**.

Without elevation the app still works and **degrades safely**: the camera is left
untouched and the microphone is still muted. Microphone mute (WASAPI) does **not**
require elevation.

---

## 6. Where your data lives

| What | Location (Windows) |
| ---- | ------------------ |
| Settings | `%APPDATA%\com.automute.walkaway\config.json` |
| App log | `%LOCALAPPDATA%\com.automute.walkaway\logs\amow.log` |
| Crash log | `%LOCALAPPDATA%\com.automute.walkaway\logs\crash.log` |

The settings file is plain JSON and safe to hand-edit; unknown or missing fields
fall back to defaults. Logs contain only short status text — never media or
personal data.

---

## 7. Troubleshooting

- **The app exits unexpectedly.** Check `crash.log` (path above). It records the
  thread, source location, panic message, and a backtrace for the last crash.
- **Mic doesn't mute.** Confirm a default *communications* capture device is set
  in Windows Sound settings; the app mutes that endpoint.
- **Camera doesn't turn off.** Run the app **as administrator** (see §5). Verify
  the webcam appears under the *Cameras* class in Device Manager.
- **No automatic presence.** The Python sidecar isn't running — install its
  `[camera]` extras, or use the manual **Away** toggle. Set
  `AMOW_PRESENCE_DISABLE=1` to skip the sidecar entirely.
- **Autostart didn't take effect.** Some security software blocks writes to the
  `Run` key; re-toggle the setting, or add the app to the allow-list.

---

## 8. Uninstall

Use **Settings → Apps → Installed apps → Auto-Mute on Walkaway → Uninstall**, or
the Start-menu uninstaller. Removing the app does not delete your `config.json`
or logs; delete the folders in §6 to remove those too.
