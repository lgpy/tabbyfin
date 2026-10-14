use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Jellyfin's time unit: 1 tick = 100 ns. Used for positions and runtimes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ticks(pub i64);

impl Ticks {
    pub const PER_SECOND: i64 = 10_000_000;
    pub const ZERO: Ticks = Ticks(0);

    pub fn from_secs_f64(secs: f64) -> Self {
        Ticks((secs * Self::PER_SECOND as f64).round() as i64)
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / Self::PER_SECOND as f64
    }
}

impl From<Duration> for Ticks {
    fn from(d: Duration) -> Self {
        Ticks(i64::try_from(d.as_nanos() / 100).unwrap_or(i64::MAX))
    }
}

impl From<Ticks> for Duration {
    /// Negative ticks (seen from some servers for unknown positions) clamp to zero.
    fn from(t: Ticks) -> Self {
        Duration::from_nanos(t.0.max(0) as u64 * 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        let d = Duration::from_millis(90_500);
        let t = Ticks::from(d);
        assert_eq!(t, Ticks(905_000_000));
        assert_eq!(Duration::from(t), d);
        assert_eq!(Ticks::from_secs_f64(1.5), Ticks(15_000_000));
        assert_eq!(Ticks(15_000_000).as_secs_f64(), 1.5);
        assert_eq!(Duration::from(Ticks(-5)), Duration::ZERO);
    }
}
