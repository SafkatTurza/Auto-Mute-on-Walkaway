"""The detection loop — wires a face source, the state machine and a bus.

The runner owns only orchestration: sample the source, advance the state
machine, publish any resulting event, and pace the loop to the target frame
rate. It contains no presence *policy* (that is the state machine) and no
device *control* (that is the host). A capture failure is logged and the tick
skipped, so a broken camera never masquerades as the user being away.
"""

from __future__ import annotations

import logging
import threading
import time

from .clock import Clock, MonotonicClock
from .config import DetectorConfig
from .event_bus import EventBus
from .events import PresenceEvent
from .face_source import FaceSource, FaceSourceError
from .state_machine import PresenceStateMachine

logger = logging.getLogger(__name__)


class DetectorRunner:
    """Drives the sample → decide → publish loop at ``config.target_fps``."""

    def __init__(
        self,
        source: FaceSource,
        bus: EventBus,
        config: DetectorConfig,
        clock: Clock | None = None,
        machine: PresenceStateMachine | None = None,
    ) -> None:
        self._source = source
        self._bus = bus
        self._config = config
        self._clock = clock if clock is not None else MonotonicClock()
        self._machine = machine or PresenceStateMachine(
            away_grace_ms=config.away_grace_ms,
            return_grace_ms=config.return_grace_ms,
        )
        self._stop = threading.Event()

    def tick(self) -> PresenceEvent | None:
        """Run exactly one sample→decide→publish cycle.

        Returns the published event, or ``None`` if the phase did not change or
        the sample was skipped due to a capture failure. Isolated from the loop
        so it can be unit-tested with fakes.
        """
        try:
            face_present = self._source.is_face_present()
        except FaceSourceError as exc:
            logger.warning("skipping tick: %s", exc)
            return None

        event = self._machine.observe(face_present, self._clock.now_ms())
        if event is not None:
            self._bus.publish(event)
        return event

    def run(self) -> None:
        """Loop until :meth:`stop` is called, pacing to the target frame rate.

        Pacing subtracts the work time from the frame budget, so the *effective*
        rate holds near the target even as per-frame detection cost varies.
        """
        interval = self._config.frame_interval_s
        logger.info("detector started at %.1f FPS", self._config.target_fps)
        while not self._stop.is_set():
            started = time.monotonic()
            try:
                self.tick()
            except Exception:  # never let one bad frame kill the loop
                logger.exception("unexpected error during detection tick")
            remaining = interval - (time.monotonic() - started)
            if remaining > 0:
                # `wait` returns early if stop() is signalled, for prompt exit.
                self._stop.wait(remaining)
        logger.info("detector stopped")

    def stop(self) -> None:
        """Signal the loop to exit after the current tick."""
        self._stop.set()
