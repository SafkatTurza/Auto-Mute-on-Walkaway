"""Tests for the pure presence state machine — no camera, injected time."""

from __future__ import annotations

import unittest

from amow_presence.events import PresenceState
from amow_presence.state_machine import PresenceStateMachine


def machine() -> PresenceStateMachine:
    # Small graces keep the arithmetic in the tests obvious.
    return PresenceStateMachine(away_grace_ms=1_000, return_grace_ms=500)


class StateMachineTests(unittest.TestCase):
    def test_starts_present(self) -> None:
        self.assertEqual(machine().state, PresenceState.PRESENT)

    def test_rejects_non_positive_grace(self) -> None:
        with self.assertRaises(ValueError):
            PresenceStateMachine(away_grace_ms=0, return_grace_ms=500)

    def test_present_face_emits_nothing(self) -> None:
        m = machine()
        self.assertIsNone(m.observe(True, 0))
        self.assertIsNone(m.observe(True, 100))
        self.assertEqual(m.state, PresenceState.PRESENT)

    def test_face_lost_enters_leaving_immediately(self) -> None:
        m = machine()
        event = m.observe(False, 0)
        assert event is not None
        self.assertEqual(event.state, PresenceState.LEAVING)
        self.assertEqual(event.at_ms, 0)
        self.assertEqual(m.state, PresenceState.LEAVING)

    def test_leaving_confirms_away_after_grace(self) -> None:
        m = machine()
        m.observe(False, 0)  # -> LEAVING
        self.assertIsNone(m.observe(False, 999))  # still within grace
        event = m.observe(False, 1_000)
        assert event is not None
        self.assertEqual(event.state, PresenceState.AWAY)

    def test_leaving_reverts_to_present_on_face_return(self) -> None:
        # A single dropped frame surfaces as LEAVING then PRESENT, never AWAY.
        m = machine()
        m.observe(False, 0)  # -> LEAVING
        event = m.observe(True, 200)
        assert event is not None
        self.assertEqual(event.state, PresenceState.PRESENT)
        self.assertEqual(m.state, PresenceState.PRESENT)

    def test_away_face_seen_enters_returning(self) -> None:
        m = machine()
        m.observe(False, 0)
        m.observe(False, 1_000)  # -> AWAY
        event = m.observe(True, 1_000)
        assert event is not None
        self.assertEqual(event.state, PresenceState.RETURNING)

    def test_returning_confirms_present_after_grace(self) -> None:
        m = machine()
        m.observe(False, 0)
        m.observe(False, 1_000)  # AWAY
        m.observe(True, 1_000)  # RETURNING
        self.assertIsNone(m.observe(True, 1_499))
        event = m.observe(True, 1_500)
        assert event is not None
        self.assertEqual(event.state, PresenceState.PRESENT)

    def test_returning_reverts_to_away_on_face_loss(self) -> None:
        m = machine()
        m.observe(False, 0)
        m.observe(False, 1_000)  # AWAY
        m.observe(True, 1_000)  # RETURNING
        event = m.observe(False, 1_100)
        assert event is not None
        self.assertEqual(event.state, PresenceState.AWAY)

    def test_away_stays_silent_without_face(self) -> None:
        m = machine()
        m.observe(False, 0)
        m.observe(False, 1_000)  # AWAY
        self.assertIsNone(m.observe(False, 2_000))
        self.assertIsNone(m.observe(False, 5_000))

    def test_transition_fires_once(self) -> None:
        m = machine()
        m.observe(False, 0)
        self.assertEqual(m.observe(False, 1_000).state, PresenceState.AWAY)  # type: ignore[union-attr]
        self.assertIsNone(m.observe(False, 2_000))

    def test_grace_timer_restarts_not_resumes(self) -> None:
        # LEAVING cancelled by a face, then lost again: the away grace restarts.
        m = machine()
        m.observe(False, 0)  # LEAVING @0
        m.observe(True, 200)  # -> PRESENT (cancel)
        m.observe(False, 300)  # LEAVING @300
        self.assertIsNone(m.observe(False, 1_299))
        event = m.observe(False, 1_300)
        assert event is not None
        self.assertEqual(event.state, PresenceState.AWAY)

    def test_non_monotonic_time_does_not_fire_early(self) -> None:
        m = machine()
        m.observe(False, 1_000)  # LEAVING, pending since 1000
        # Clock appears to step backwards; elapsed clamps to 0.
        self.assertIsNone(m.observe(False, 500))
        self.assertEqual(m.state, PresenceState.LEAVING)

    def test_full_walkaway_and_return_cycle(self) -> None:
        m = machine()
        states = []
        script = [
            (True, 0),
            (False, 100),  # LEAVING
            (False, 1_100),  # AWAY
            (True, 2_000),  # RETURNING
            (True, 2_600),  # PRESENT (2000 + 500 grace reached at 2500)
        ]
        for face, t in script:
            ev = m.observe(face, t)
            if ev is not None:
                states.append(ev.state)
        self.assertEqual(
            states,
            [
                PresenceState.LEAVING,
                PresenceState.AWAY,
                PresenceState.RETURNING,
                PresenceState.PRESENT,
            ],
        )


if __name__ == "__main__":
    unittest.main()
