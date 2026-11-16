//! The monotonic time source the circuit breaker measures its cooldown against.
//!
//! The breaker's open→half-open transition is a *duration* comparison ("has the
//! cooldown elapsed?"), never a wall-clock one, so it reads time through a
//! [`TimeSource`] rather than calling [`Instant::now`] inline. Production uses
//! [`SystemTimeSource`] (milliseconds since a base `Instant`, monotonic and
//! immune to wall-clock jumps); a test uses [`ManualTimeSource`] to advance the
//! clock with no real sleeping, so the cooldown transition is exercised instantly
//! and deterministically.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// A monotonic clock reporting milliseconds from an arbitrary, fixed epoch. Only
/// *differences* between readings are meaningful; the absolute value carries no
/// wall-clock meaning. Readings never go backwards.
pub trait TimeSource: Send + Sync {
    /// The current reading, in milliseconds from this source's epoch.
    fn now_millis(&self) -> u64;
}

/// The production clock: milliseconds since the source captured its base
/// [`Instant`] at construction. Monotonic and independent of any wall-clock
/// adjustment (NTP steps, leap seconds, operator changes).
#[derive(Debug)]
pub struct SystemTimeSource {
    base: Instant,
}

impl SystemTimeSource {
    /// A source whose epoch is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
        }
    }
}

impl Default for SystemTimeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeSource for SystemTimeSource {
    fn now_millis(&self) -> u64 {
        // `elapsed()` is monotonic; the cast saturates far beyond any realistic
        // uptime (u64 ms ≈ 584 million years).
        self.base.elapsed().as_millis() as u64
    }
}

/// A manually-advanced clock for deterministic tests: [`advance_millis`] moves it
/// forward with no real sleeping, so the breaker's cooldown-driven transitions
/// are exercised instantly instead of waiting out a real timer.
///
/// [`advance_millis`]: ManualTimeSource::advance_millis
#[derive(Debug, Default)]
pub struct ManualTimeSource {
    millis: AtomicU64,
}

impl ManualTimeSource {
    /// A source reading zero until advanced.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the clock by `delta` milliseconds.
    pub fn advance_millis(&self, delta: u64) {
        self.millis.fetch_add(delta, Ordering::SeqCst);
    }
}

impl TimeSource for ManualTimeSource {
    fn now_millis(&self) -> u64 {
        self.millis.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_source_starts_at_zero_and_advances() {
        let clock = ManualTimeSource::new();
        assert_eq!(clock.now_millis(), 0);
        clock.advance_millis(250);
        assert_eq!(clock.now_millis(), 250);
        clock.advance_millis(750);
        assert_eq!(clock.now_millis(), 1000);
    }

    #[test]
    fn system_source_is_monotonic() {
        let clock = SystemTimeSource::new();
        let a = clock.now_millis();
        let b = clock.now_millis();
        assert!(b >= a, "a monotonic clock never goes backwards");
    }
}
