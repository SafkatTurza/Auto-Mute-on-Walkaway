"""Tests for the detection loop using fake source/clock/bus (no camera)."""

from __future__ import annotations

import unittest

from amow_presence.config import DetectorConfig
from amow_presence.event_bus import InMemoryEventBus
from amow_presence.events import PresenceState
from amow_presence.face_source import FaceSourceError
from amow_presence.runner import DetectorRunner


class FakeClock:
    """Clock whose value is advanced explicitly by the test."""

    def __init__(self) -> None:
        self.value = 0

    def now_ms(self) -> int:
        return self.value


class ScriptedSource:
    """Yields a scripted sequence of face-present booleans.

    A boolean value means "face present / absent"; the special sentinel
    ``ERROR`` raises :class:`FaceSourceError` to model a capture failure.
    """

    ERROR = object()

    def __init__(self, script: list[object]) -> None:
        self._script = list(script)
        self.closed = False

    def is_face_present(self) -> bool:
        item = self._script.pop(0)
        if item is ScriptedSource.ERROR:
            raise FaceSourceError("simulated capture failure")
        return bool(item)

    def close(self) -> None:
        self.closed = True


def config() -> DetectorConfig:
    return DetectorConfig(away_grace_ms=1_000, return_grace_ms=500, target_fps=15.0)


class RunnerTickTests(unittest.TestCase):
    def test_tick_publishes_transition_events(self) -> None:
        clock = FakeClock()
        source = ScriptedSource([True, False, False, False])
        bus = InMemoryEventBus()
        runner = DetectorRunner(source=source, bus=bus, config=config(), clock=clock)

        clock.value = 0
        self.assertIsNone(runner.tick())  # present, no change
        clock.value = 100
        self.assertEqual(runner.tick().state, PresenceState.LEAVING)  # type: ignore[union-attr]
        clock.value = 1_000
        self.assertIsNone(runner.tick())  # still within away grace
        clock.value = 1_100
        self.assertEqual(runner.tick().state, PresenceState.AWAY)  # type: ignore[union-attr]

        self.assertEqual(
            [e.state for e in bus.events],
            [PresenceState.LEAVING, PresenceState.AWAY],
        )

    def test_capture_failure_is_skipped_not_treated_as_away(self) -> None:
        clock = FakeClock()
        # A capture error must not advance the machine toward AWAY.
        source = ScriptedSource([ScriptedSource.ERROR, ScriptedSource.ERROR])
        bus = InMemoryEventBus()
        runner = DetectorRunner(source=source, bus=bus, config=config(), clock=clock)

        clock.value = 5_000
        self.assertIsNone(runner.tick())
        clock.value = 10_000
        self.assertIsNone(runner.tick())
        self.assertEqual(bus.events, [])

    def test_stop_ends_the_loop(self) -> None:
        source = ScriptedSource([True])
        runner = DetectorRunner(source=source, bus=InMemoryEventBus(), config=config())
        runner.stop()
        runner.run()  # returns immediately because stop is already set


if __name__ == "__main__":
    unittest.main()
