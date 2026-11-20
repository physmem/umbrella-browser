use std::time::{Duration, Instant};

pub const SESSION_SAVE_DELAY: Duration = Duration::from_millis(500);

/// Coalesces bursts of changes into one write. The caller owns the clock so
/// this works on any thread or event loop without a timer of its own.
#[derive(Debug, Clone)]
pub struct Debouncer {
    delay: Duration,
    last_change: Option<Instant>,
}

impl Debouncer {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            last_change: None,
        }
    }

    pub fn mark_dirty(&mut self, now: Instant) {
        self.last_change = Some(now);
    }

    pub fn is_dirty(&self) -> bool {
        self.last_change.is_some()
    }

    /// When the pending write should fire, if any.
    pub fn deadline(&self) -> Option<Instant> {
        self.last_change.map(|t| t + self.delay)
    }

    /// True once per burst, after the delay has passed since the last change.
    pub fn take_due(&mut self, now: Instant) -> bool {
        let due = self.deadline().is_some_and(|d| now >= d);
        if due {
            self.last_change = None;
        }
        due
    }

    /// For flush-on-close: true if anything was pending, regardless of delay.
    pub fn take_pending(&mut self) -> bool {
        self.last_change.take().is_some()
    }
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::new(SESSION_SAVE_DELAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_after_quiet_period() {
        let start = Instant::now();
        let mut d = Debouncer::default();
        d.mark_dirty(start);
        d.mark_dirty(start + Duration::from_millis(300));
        assert!(!d.take_due(start + Duration::from_millis(600)));
        assert!(d.take_due(start + Duration::from_millis(800)));
        assert!(!d.take_due(start + Duration::from_millis(900)));
    }

    #[test]
    fn flush_takes_pending_early() {
        let mut d = Debouncer::default();
        assert!(!d.take_pending());
        d.mark_dirty(Instant::now());
        assert!(d.take_pending());
        assert!(!d.is_dirty());
    }
}
