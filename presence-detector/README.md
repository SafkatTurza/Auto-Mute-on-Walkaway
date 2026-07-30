# Presence Detector (`amow_presence`)

A modular, privacy-first webcam **presence detector** for Auto-Mute on Walkaway.
It watches the local webcam, decides *where the user is in the walk-away
lifecycle*, and emits one of four events. That is its entire job.

```
present  ─face lost→  leaving  ─away grace→  away  ─face seen→  returning  ─return grace→  present
```

## Scope — what this module does *not* do

By design, and to keep it free of business logic:

- **No device control.** It never mutes a microphone or disables a camera. It
  only reports presence; the host application owns every action.
- **No policy.** It does not know what a "meeting" is, or whether muting is
  enabled. It just classifies presence.
- **No network.** All detection runs on-device. No frames, images, audio, or
  personal data ever leave the machine — nothing is stored or uploaded.

## Architecture

Ports-and-adapters, mirroring the Rust side of the project. The presence
**state machine is pure logic** with time injected, so every transition is
deterministic and unit-tested without a camera. OpenCV and MediaPipe are
quarantined behind a single adapter and imported lazily.

```
face_source.py   FaceSource port + MediaPipeFaceSource (OpenCV capture + MediaPipe)
state_machine.py PresenceStateMachine — pure debouncing logic, injected clock
events.py        PresenceState (present/leaving/away/returning) + PresenceEvent
event_bus.py     EventBus port + StdoutEventBus (sidecar) + InMemoryEventBus (tests)
clock.py         Clock port + MonotonicClock
config.py        DetectorConfig — all thresholds, validated
runner.py        DetectorRunner — the paced sample→decide→publish loop
__main__.py      CLI entry point
```

## The Event Bus contract (sidecar integration)

The detector publishes onto an `EventBus`. In production that is
`StdoutEventBus`, which writes **one compact JSON object per line to stdout**:

```json
{"type":"presence","state":"leaving","at_ms":8421}
{"type":"presence","state":"away","at_ms":11421}
{"type":"presence","state":"returning","at_ms":19004}
{"type":"presence","state":"present","at_ms":19804}
```

`at_ms` is a monotonic millisecond timestamp (immune to clock jumps). All logs
go to **stderr**, so stdout carries only events. The Tauri/Rust host runs this
as a sidecar process and bridges each line onto its own in-process `EventBus`
(mapping `away`/`present` to the domain `PresenceState`, and treating the
transitional `leaving`/`returning` as advisory) — so the detector plugs into the
same event-driven seam as the rest of the app without importing any app code.

## Configuration

Thresholds live in `DetectorConfig` and are validated on construction:

| Setting                    | Default | Meaning                                   |
| -------------------------- | ------- | ----------------------------------------- |
| `away_grace_ms`            | 3000    | Face absent this long → `away`            |
| `return_grace_ms`          | 800     | Face present this long → `present`        |
| `target_fps`               | 15.0    | Detection rate; caps CPU cost             |
| `camera_index`             | 0       | Which webcam to open                      |
| `min_detection_confidence` | 0.5     | MediaPipe face-detection threshold        |
| `model_selection`          | 0       | 0 = short-range (cheap), 1 = full-range   |

The two grace periods are **shared** with the app's `config.json` (the
`presence` block); `DetectorConfig.from_app_config_file()` reads just those keys
and ignores everything else, so it never clashes with the Rust schema.

## Low CPU

~15 FPS is the default because face detection dominates cost, so capping the
frame rate caps CPU. The loop subtracts per-frame work time from the frame
budget to hold the effective rate steady, and MediaPipe's short-range model is
used by default. A capture failure is logged and the tick skipped — a broken
camera is never misread as "the user left".

## Running

```bash
# Install the live-camera extras (only needed to use a real webcam):
pip install -e ".[camera]"

# Run against the app config, streaming events to stdout:
python -m amow_presence --config ~/.config/com.automute.walkaway/config.json

# Or standalone with explicit knobs:
python -m amow_presence --camera 0 --fps 15 --confidence 0.5
```

Ctrl-C / SIGTERM stops cleanly and releases the camera.

## Testing & linting

The test suite covers the state machine, event bus, config, and runner using
fakes — **no camera or CV libraries required**:

```bash
pip install -e ".[dev]"
pytest            # unit tests
ruff check .      # lint
mypy amow_presence
```
