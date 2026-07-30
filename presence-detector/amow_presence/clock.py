"""Monotonic clock port and its default implementation.

Presence timing must be immune to wall-clock jumps (NTP corrections, DST, a
user changing the system time), so the detector reasons purely in monotonic
milliseconds. Injecting the clock also makes the state machine deterministic
under test.
"""

from __future__ import annotations

import time
from typing import Protocol, runtime_checkable


@runtime_checkable
class Clock(Protocol):
    """Source of monotonic milliseconds since some fixed, arbitrary origin."""

    def now_ms(self) -> int: ...


class MonotonicClock:
    """Default clock backed by :func:`time.monotonic_ns`.

    The origin is captured at construction so values start near zero, keeping
    emitted timestamps small and readable.
    """

    def __init__(self) -> None:
        self._base_ns = time.monotonic_ns()

    def now_ms(self) -> int:
        return (time.monotonic_ns() - self._base_ns) // 1_000_000
