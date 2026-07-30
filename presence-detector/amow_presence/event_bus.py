"""The Event Bus port and the two transports the detector ships with.

The detector publishes every transition onto an :class:`EventBus`. It never
knows *who* is listening — this is the seam that keeps it decoupled from the
host. Two implementations are provided:

* :class:`StdoutEventBus` — the production transport. It writes one compact
  JSON object per line to a stream (stdout by default). The Tauri/Rust host runs
  the detector as a sidecar process and bridges these lines onto its own
  in-process ``EventBus``. Diagnostics must therefore go to *stderr*, never
  stdout, so the event channel stays clean.
* :class:`InMemoryEventBus` — for tests and in-process consumers; it records
  events and fans them out to optional subscribers.
"""

from __future__ import annotations

import sys
from collections.abc import Callable
from typing import Protocol, TextIO, runtime_checkable

from .events import PresenceEvent


@runtime_checkable
class EventBus(Protocol):
    """Anything the detector can publish presence events to."""

    def publish(self, event: PresenceEvent) -> None: ...


class StdoutEventBus:
    """Publishes events as newline-delimited JSON on a text stream.

    This is the sidecar contract: each line is a self-contained JSON object the
    host can parse independently. The stream is flushed per event so the host
    sees transitions with minimal latency.
    """

    def __init__(self, stream: TextIO | None = None) -> None:
        self._stream = stream if stream is not None else sys.stdout

    def publish(self, event: PresenceEvent) -> None:
        self._stream.write(event.to_json() + "\n")
        self._stream.flush()


class InMemoryEventBus:
    """Collects events and fans out to subscribers; primarily for tests."""

    def __init__(self) -> None:
        self.events: list[PresenceEvent] = []
        self._subscribers: list[Callable[[PresenceEvent], None]] = []

    def subscribe(self, callback: Callable[[PresenceEvent], None]) -> None:
        self._subscribers.append(callback)

    def publish(self, event: PresenceEvent) -> None:
        self.events.append(event)
        for callback in list(self._subscribers):
            callback(event)
