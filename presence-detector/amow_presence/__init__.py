"""Auto-Mute on Walkaway — webcam presence detector.

A modular, privacy-first sidecar that watches the webcam locally and emits
``present`` / ``leaving`` / ``away`` / ``returning`` events. It holds no
business logic and controls no devices: it only reports where the user is in
the walk-away lifecycle, leaving every decision to the host application.

Everything runs on-device. No frames, images, audio or personal data ever
leave the machine.
"""

from __future__ import annotations

from .clock import Clock, MonotonicClock
from .config import DetectorConfig
from .event_bus import EventBus, InMemoryEventBus, StdoutEventBus
from .events import PresenceEvent, PresenceState
from .face_source import FaceSource, FaceSourceError, MediaPipeFaceSource
from .runner import DetectorRunner
from .state_machine import PresenceStateMachine

__all__ = [
    "Clock",
    "MonotonicClock",
    "DetectorConfig",
    "EventBus",
    "InMemoryEventBus",
    "StdoutEventBus",
    "PresenceEvent",
    "PresenceState",
    "FaceSource",
    "FaceSourceError",
    "MediaPipeFaceSource",
    "DetectorRunner",
    "PresenceStateMachine",
]

__version__ = "0.1.0"
