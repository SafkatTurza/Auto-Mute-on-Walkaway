"""Presence events — the sole vocabulary this module emits.

The detector reports *where the user is in the walk-away lifecycle* and nothing
else. It never decides to mute, disable a camera, or take any action: those are
the host application's job. Keeping this module's output limited to these four
phases is what keeps it free of business logic.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import Enum


class PresenceState(str, Enum):
    """The user's phase in the presence lifecycle.

    ``LEAVING`` and ``RETURNING`` are the transitional grace windows: the face
    has just been lost / regained but the change has not yet been confirmed.
    They let a consumer show "hold on" feedback without acting prematurely.
    """

    PRESENT = "present"
    LEAVING = "leaving"
    AWAY = "away"
    RETURNING = "returning"


@dataclass(frozen=True)
class PresenceEvent:
    """A single presence transition, carrying the monotonic time it occurred.

    ``at_ms`` is a monotonic millisecond count (never wall-clock), so consumers
    can reason about durations without being affected by NTP or DST jumps.
    """

    state: PresenceState
    at_ms: int

    def to_dict(self) -> dict[str, object]:
        return {"type": "presence", "state": self.state.value, "at_ms": self.at_ms}

    def to_json(self) -> str:
        """Compact single-line JSON — the on-the-wire form for the sidecar bus."""
        return json.dumps(self.to_dict(), separators=(",", ":"))
