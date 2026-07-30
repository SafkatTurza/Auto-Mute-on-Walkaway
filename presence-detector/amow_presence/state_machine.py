"""The presence state machine — pure logic, no I/O, injected time.

This is the heart of the detector and it is deliberately free of OpenCV,
MediaPipe, threads and clocks. It consumes a stream of raw ``face_present``
booleans plus a monotonic timestamp and produces at most one
:class:`PresenceEvent` per tick, exactly on the ticks where the phase changes.
Because time is injected, every transition is deterministic under test.

Phases and the transitions between them::

    PRESENT --face lost--> LEAVING --grace elapsed--> AWAY
       ^                     |                          |
       | face regained       | face regained            | face seen
       |  (return grace)     v (false alarm)            v
    RETURNING <----------- PRESENT                    RETURNING

``LEAVING`` runs the *away* grace timer; ``RETURNING`` runs the *return* grace
timer. A disagreeing sample during a transitional phase reverts to the last
committed phase (``PRESENT`` / ``AWAY``) and emits that as its own event, so a
single dropped detection frame surfaces as a brief ``LEAVING``/``PRESENT`` blip
rather than a spurious ``AWAY``.
"""

from __future__ import annotations

from .events import PresenceEvent, PresenceState


class PresenceStateMachine:
    """Debounces raw face samples into stable presence-phase transitions.

    Starts in :attr:`PresenceState.PRESENT` — the safe default: the user is
    assumed at the desk until proven otherwise, so nothing is ever triggered on
    start-up.
    """

    def __init__(self, away_grace_ms: int, return_grace_ms: int) -> None:
        if away_grace_ms <= 0 or return_grace_ms <= 0:
            raise ValueError("grace periods must be positive")
        self._away_grace_ms = away_grace_ms
        self._return_grace_ms = return_grace_ms
        self._state = PresenceState.PRESENT
        # When the current transitional phase began, or None when committed.
        self._pending_since: int | None = None

    @property
    def state(self) -> PresenceState:
        """The current committed or transitional phase."""
        return self._state

    def observe(self, face_present: bool, now_ms: int) -> PresenceEvent | None:
        """Feed one detection sample; return an event only when the phase flips.

        ``now_ms`` is the monotonic time of this sample. Returns ``None`` on
        ticks that do not change the phase.
        """
        new_state = self._next_state(face_present, now_ms)
        if new_state is None:
            return None
        self._state = new_state
        return PresenceEvent(state=new_state, at_ms=now_ms)

    def _next_state(self, face_present: bool, now_ms: int) -> PresenceState | None:
        if self._state is PresenceState.PRESENT:
            if face_present:
                return None
            self._pending_since = now_ms
            return PresenceState.LEAVING

        if self._state is PresenceState.LEAVING:
            if face_present:
                self._pending_since = None
                return PresenceState.PRESENT
            if self._elapsed(now_ms) >= self._away_grace_ms:
                self._pending_since = None
                return PresenceState.AWAY
            return None

        if self._state is PresenceState.AWAY:
            if not face_present:
                return None
            self._pending_since = now_ms
            return PresenceState.RETURNING

        # RETURNING
        if not face_present:
            self._pending_since = None
            return PresenceState.AWAY
        if self._elapsed(now_ms) >= self._return_grace_ms:
            self._pending_since = None
            return PresenceState.PRESENT
        return None

    def _elapsed(self, now_ms: int) -> int:
        """Time in the current transitional phase, clamped at 0.

        The clamp guards against a non-monotonic ``now_ms`` (e.g. a clock that
        appears to step backwards) so a grace timer can never fire early.
        """
        if self._pending_since is None:
            return 0
        return max(0, now_ms - self._pending_since)
