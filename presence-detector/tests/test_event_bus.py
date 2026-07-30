"""Tests for the event bus transports."""

from __future__ import annotations

import io
import json
import unittest

from amow_presence.event_bus import InMemoryEventBus, StdoutEventBus
from amow_presence.events import PresenceEvent, PresenceState


class StdoutEventBusTests(unittest.TestCase):
    def test_writes_one_json_line_per_event(self) -> None:
        stream = io.StringIO()
        bus = StdoutEventBus(stream=stream)
        bus.publish(PresenceEvent(PresenceState.LEAVING, 1))
        bus.publish(PresenceEvent(PresenceState.AWAY, 2))

        lines = stream.getvalue().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertEqual(json.loads(lines[0])["state"], "leaving")
        self.assertEqual(json.loads(lines[1])["state"], "away")


class InMemoryEventBusTests(unittest.TestCase):
    def test_records_events(self) -> None:
        bus = InMemoryEventBus()
        bus.publish(PresenceEvent(PresenceState.PRESENT, 0))
        self.assertEqual([e.state for e in bus.events], [PresenceState.PRESENT])

    def test_fans_out_to_subscribers(self) -> None:
        bus = InMemoryEventBus()
        seen: list[PresenceState] = []
        bus.subscribe(lambda e: seen.append(e.state))
        bus.publish(PresenceEvent(PresenceState.RETURNING, 3))
        self.assertEqual(seen, [PresenceState.RETURNING])


if __name__ == "__main__":
    unittest.main()
