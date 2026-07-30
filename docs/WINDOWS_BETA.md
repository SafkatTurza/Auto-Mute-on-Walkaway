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

- **Python 3.9–3.12** (MediaPipe has no wheels for 3.13+) with the detector's
  extras: `pip install -e "presence-detector[camera]"` (pulls in `opencv-python`
  and `mediapipe`). Without it, use the manual Away toggle.
- The detector package itself is **bundled into the installer**, and the app
  **auto-discovers** a suitable Python (it prefers the `py -3.12` launcher and
  an interpreter that can import the dependencies). So once Python and the extras
  are installed, presence detection works on a **plain launch** — no script and
  no environment variables required. `AMOW_PRESENCE_PYTHON` /
  `AMOW_PRESENCE_DIR` still override the discovery if you need a specific setup.

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
   - The status panel shows the live **Source** — *📷 Webcam (auto)* when the
     sidecar is running, or *✋ Manual* when it isn't — so you can tell at a
     glance which is driving presence. The manual card dims itself while the
     webcam is active.

> **Tip — running elevated for camera control.** Presence detection works on a
> plain launch (the app bundles the detector and finds Python itself). The
> `./run-with-presence.ps1` helper is still handy when you want to run **as
> administrator** so the camera can be disabled: `./run-with-presence.ps1
> -AsAdmin`. It can also `-Install` the detector's Python dependencies, pin a
> specific interpreter, or `-Dev` to run from source with `npm run tauri dev`.
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

## 5. Camera control is opt-in and needs elevation

**Camera disabling is off by default.** Out of the box the app only mutes your
microphone on walkaway — that uses WASAPI, needs no special rights, and is always
reversible. Turn on **Automatic actions → Disable camera** to opt in.

Disabling a camera device toggles its device node (the same action as Device
Manager's *Disable device*), which requires **administrator privileges** in both
directions — to switch it off *and* to switch it back on. To use camera control,
run the app **as administrator** (the bundled `run-with-presence.ps1 -AsAdmin`
does this for you).

**The app never disables a camera it could not turn back on.** If camera control
is enabled but the app is *not* running elevated, it leaves the camera untouched
and flags it in the UI ("run as administrator") rather than switching off a
device it has no privilege to restore. The microphone is still muted either way.

### Returning when the camera is disabled

There is a catch to disabling the camera while presence comes from the webcam: a
disabled camera produces no frames, so the webcam **cannot see you come back**.
To handle that, while the camera is disabled the app watches for **keyboard or
mouse activity** as a camera-free "you're back" signal — the moment you use the
computer, it re-enables the camera and un-mutes the mic, and webcam presence
takes over again.

This reads only *idle time* (seconds since the last input) — never what you type
— and it only ever signals *return*, never *away*, so it can bring you back but
can never mute you. (For automatic restore to happen, keep **Restore on return**
on.) If you prefer to avoid this entirely, leave **Disable camera** off and use
webcam presence with mic-mute only — then the camera stays on and sees you return
directly.

### The camera is never left disabled

Whenever the app disables your webcam, it re-enables it in every exit path:

- **Return / Protection off** — restored immediately.
- **Quitting the app** (tray *Quit* or closing the window) — restored before the
  process exits, even if you had *Restore on return* switched off.
- **Crash, force-kill, or power loss** — the app records the devices it disabled
  in `camera-recovery.txt` (next to `config.json`) and **re-enables them
  automatically on the next launch**, before anything else runs. If that launch
  is *not* elevated (so recovery can't run), the app tells you with a desktop
  notification — **"Camera still disabled"** — instead of failing silently.
  Relaunch as administrator, or re-enable the webcam in Device Manager.

It only ever re-enables cameras it disabled itself — a webcam you turned off in
Device Manager before starting the app is left exactly as you set it.

---

## 6. Where your data lives

| What | Location (Windows) |
| ---- | ------------------ |
| Settings | `%APPDATA%\com.automute.walkaway\config.json` |
| Camera recovery record | `%APPDATA%\com.automute.walkaway\camera-recovery.txt` |
| App log | `%LOCALAPPDATA%\com.automute.walkaway\logs\amow.log` |
| Crash log | `%LOCALAPPDATA%\com.automute.walkaway\logs\crash.log` |

The camera-recovery record lists only opaque device ids of cameras the app has
currently disabled; it exists only while a camera is off and is removed once the
camera is restored.

The settings file is plain JSON and safe to hand-edit; unknown or missing fields
fall back to defaults. Logs contain only short status text — never media or
personal data.

---

## 7. Troubleshooting

- **The app exits unexpectedly.** Check `crash.log` (path above). It records the
  thread, source location, panic message, and a backtrace for the last crash.
- **Mic doesn't mute.** Confirm a default *communications* capture device is set
  in Windows Sound settings; the app mutes that endpoint.
- **Camera doesn't turn off.** Camera control is opt-in — enable *Automatic
  actions → Disable camera* — and run the app **as administrator** (see §5).
  Verify the webcam appears under the *Cameras* class in Device Manager.
- **Camera is stuck disabled / won't turn back on.** Open **Device Manager →
  Cameras**, right-click your webcam and choose **Enable device**. (If that is
  greyed out or it stays dead, choose **Uninstall device** — *without* deleting
  the driver — then **Action → Scan for hardware changes**, or reboot.) Then
  relaunch Auto-Mute **as administrator** so its automatic recovery can run; the
  app re-enables cameras it disabled on the next elevated launch.
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
