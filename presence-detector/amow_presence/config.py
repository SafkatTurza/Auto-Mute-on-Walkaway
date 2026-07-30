"""Detector configuration — all thresholds live here and are validated once.

The presence grace periods are shared with the Rust application: they live in
the app's ``config.json`` under the ``presence`` block. The detector reads that
file leniently (it takes only the keys it understands and ignores the rest), so
it never conflicts with the Rust side's strict schema. Detector-only knobs
(frame rate, camera index, detection confidence) are supplied separately via
CLI flags or overrides and default sensibly.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

# Defaults mirror the Rust `PresenceConfig` so both sides agree out of the box.
DEFAULT_AWAY_GRACE_MS = 3_000
DEFAULT_RETURN_GRACE_MS = 800
# 15 FPS is the sweet spot: fast enough to feel responsive, slow enough to keep
# idle CPU low. Face detection is the dominant cost, so capping the rate caps CPU.
DEFAULT_TARGET_FPS = 15.0
DEFAULT_CAMERA_INDEX = 0
DEFAULT_MIN_DETECTION_CONFIDENCE = 0.5
# MediaPipe short-range model (0) is tuned for faces within ~2m of a webcam and
# is markedly cheaper than the full-range model (1).
DEFAULT_MODEL_SELECTION = 0


@dataclass(frozen=True)
class DetectorConfig:
    """Validated, immutable detector settings."""

    away_grace_ms: int = DEFAULT_AWAY_GRACE_MS
    return_grace_ms: int = DEFAULT_RETURN_GRACE_MS
    target_fps: float = DEFAULT_TARGET_FPS
    camera_index: int = DEFAULT_CAMERA_INDEX
    min_detection_confidence: float = DEFAULT_MIN_DETECTION_CONFIDENCE
    model_selection: int = DEFAULT_MODEL_SELECTION

    def __post_init__(self) -> None:
        if self.away_grace_ms <= 0 or self.return_grace_ms <= 0:
            raise ValueError("grace periods must be > 0")
        if not 0 < self.target_fps <= 60:
            raise ValueError("target_fps must be in (0, 60]")
        if self.camera_index < 0:
            raise ValueError("camera_index must be >= 0")
        if not 0.0 <= self.min_detection_confidence <= 1.0:
            raise ValueError("min_detection_confidence must be in [0.0, 1.0]")
        if self.model_selection not in (0, 1):
            raise ValueError("model_selection must be 0 (short-range) or 1 (full-range)")

    @property
    def frame_interval_s(self) -> float:
        """Target seconds between frames, derived from the frame rate."""
        return 1.0 / self.target_fps

    @classmethod
    def from_app_config_file(cls, path: str | Path, **overrides: object) -> DetectorConfig:
        """Build a config from the app's shared ``config.json``.

        Only ``presence.away_grace_ms`` / ``presence.return_grace_ms`` are read;
        any other keys in the file are ignored. ``overrides`` (detector-only
        knobs, or grace overrides) win over the file. A missing file is not an
        error — defaults are used — because the detector can run standalone.
        """
        values: dict[str, object] = {}
        try:
            raw = json.loads(Path(path).read_text(encoding="utf-8"))
        except FileNotFoundError:
            raw = {}
        presence = raw.get("presence", {}) if isinstance(raw, dict) else {}
        if isinstance(presence, dict):
            if "away_grace_ms" in presence:
                values["away_grace_ms"] = presence["away_grace_ms"]
            if "return_grace_ms" in presence:
                values["return_grace_ms"] = presence["return_grace_ms"]
        values.update({k: v for k, v in overrides.items() if v is not None})
        return cls(**values)  # type: ignore[arg-type]
