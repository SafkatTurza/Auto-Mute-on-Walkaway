use std::time::Instant;

use amow_application::Clock;

/// Monotonic wall-clock adapter backed by [`std::time::Instant`].
///
/// The application layer requires a *monotonic* millisecond count, not a
/// calendar time: presence debouncing must never be confused by the system
/// clock jumping (NTP steps, DST, manual changes). `Instant` is guaranteed
/// non-decreasing, so elapsed-since-start is exactly what the domain wants.
pub struct SystemClock {
    base: Instant,
}

impl SystemClock {
    /// Create a clock whose zero point is now.
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        // Saturating on overflow is fine: it would take ~585 million years of
        // uptime to overflow u64 milliseconds.
        self.base.elapsed().as_millis() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_monotonic_non_decreasing() {
        let clock = SystemClock::new();
        let a = clock.now_ms();
        let b = clock.now_ms();
        assert!(b >= a, "clock went backwards: {a} -> {b}");
    }

    #[test]
    fn starts_near_zero() {
        let clock = SystemClock::new();
        // Immediately after construction the elapsed time is tiny.
        assert!(clock.now_ms() < 1_000);
    }
}
