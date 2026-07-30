"""Tests for the event value objects and their JSON wire form."""

from __future__ import annotations

import json
import unittest
from dataclasses import FrozenInstanceError

from amow_presence.events import PresenceEvent, PresenceState


class EventTests(unittest.TestCase):
    def test_to_dict_shape(self) -> None:
        event = PresenceEvent(state=PresenceState.AWAY, at_ms=1234)
        self.assertEqual(
            event.to_dict(),
            {"type": "presence", "state": "away", "at_ms": 1234},
        )

    def test_to_json_is_compact_single_line(self) -> None:
        line = PresenceEvent(state=PresenceState.LEAVING, at_ms=7).to_json()
        self.assertNotIn("\n", line)
        self.assertNotIn(" ", line)
        self.assertEqual(
            json.loads(line),
            {"type": "presence", "state": "leaving", "at_ms": 7},
        )

    def test_all_four_states_serialise(self) -> None:
        self.assertEqual(
            sorted(s.value for s in PresenceState),
            ["away", "leaving", "present", "returning"],
        )

    def test_event_is_immutable(self) -> None:
        event = PresenceEvent(state=PresenceState.PRESENT, at_ms=0)
        with self.assertRaises(FrozenInstanceError):
            event.at_ms = 5  # type: ignore[misc]


if __name__ == "__main__":
    unittest.main()
