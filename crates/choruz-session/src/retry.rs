//! Retry scheduling with exponential backoff.
//!
//! The backoff formula is: `min(2^attempt_count, MAX_BACKOFF_SECS)` seconds.
//! After `max_attempts` (default 3), the command is dead-lettered.

use chrono::{DateTime, TimeDelta, Utc};

/// Maximum backoff duration in seconds (5 minutes).
pub const MAX_BACKOFF_SECS: i64 = 300;

/// Default maximum number of attempts (including the initial run) before
/// dead-lettering. Three keeps transient recovery useful without holding an
/// agent's single-flight queue hostage for a broken local environment.
pub const DEFAULT_MAX_ATTEMPTS: i32 = 3;

/// Compute the next retry time using exponential backoff.
///
/// `attempt_count` is the number of attempts already made (1-based after the
/// first failure). The returned `DateTime` is the earliest time the command
/// should be retried.
///
/// Formula: `now + min(2^attempt_count, 300)` seconds.
pub fn next_retry_at(now: DateTime<Utc>, attempt_count: i32) -> DateTime<Utc> {
    let backoff = exponential_backoff_secs(attempt_count);
    now + TimeDelta::seconds(backoff)
}

/// Compute the backoff duration in seconds for a given attempt count.
pub fn exponential_backoff_secs(attempt_count: i32) -> i64 {
    let base: i64 = 2;
    let exponent = attempt_count.min(30) as u32; // cap exponent to prevent overflow
    base.saturating_pow(exponent).min(MAX_BACKOFF_SECS)
}

/// Returns `true` if the command has exhausted all retry attempts.
pub fn is_exhausted(attempt_count: i32, max_attempts: i32) -> bool {
    attempt_count >= max_attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_increases_exponentially() {
        assert_eq!(exponential_backoff_secs(0), 1); // 2^0 = 1
        assert_eq!(exponential_backoff_secs(1), 2); // 2^1 = 2
        assert_eq!(exponential_backoff_secs(2), 4); // 2^2 = 4
        assert_eq!(exponential_backoff_secs(3), 8); // 2^3 = 8
        assert_eq!(exponential_backoff_secs(4), 16); // 2^4 = 16
        assert_eq!(exponential_backoff_secs(5), 32); // 2^5 = 32
    }

    #[test]
    fn backoff_caps_at_max() {
        assert_eq!(exponential_backoff_secs(9), MAX_BACKOFF_SECS); // 2^9 = 512 > 300
        assert_eq!(exponential_backoff_secs(20), MAX_BACKOFF_SECS);
    }

    #[test]
    fn next_retry_at_adds_backoff_to_now() {
        let now = Utc::now();
        let retry_time = next_retry_at(now, 3);
        let expected = now + TimeDelta::seconds(8);
        assert_eq!(retry_time, expected);
    }

    #[test]
    fn is_exhausted_works() {
        assert!(!is_exhausted(0, 5));
        assert!(!is_exhausted(4, 5));
        assert!(is_exhausted(5, 5));
        assert!(is_exhausted(6, 5));
    }

    #[test]
    fn backoff_handles_large_attempt_count() {
        // Should not panic even with very large attempt counts
        assert_eq!(exponential_backoff_secs(100), MAX_BACKOFF_SECS);
    }
}
