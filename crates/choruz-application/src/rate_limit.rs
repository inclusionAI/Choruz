use choruz_common::{AppError, AppResult};
use chrono::{DateTime, Duration, Utc};

pub(crate) fn check_window(
    entries: &mut Vec<DateTime<Utc>>,
    limit: usize,
    at: DateTime<Utc>,
) -> AppResult<()> {
    let window = Duration::minutes(1);
    entries.retain(|timestamp| *timestamp > at - window);
    if entries.len() >= limit {
        // Rejected requests do not extend the window. A zero limit stays closed.
        let remaining = entries.iter().min().copied().unwrap_or(at) + window - at;
        let retry_after_ms = remaining
            .to_std()
            .unwrap_or_default()
            .as_nanos()
            .div_ceil(1_000_000) as u64;
        return Err(AppError::RateLimited { retry_after_ms });
    }
    entries.push(at);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_waits_for_the_oldest_hit_without_extending_it() {
        let start = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut entries = Vec::new();
        check_window(&mut entries, 2, start).unwrap();
        check_window(&mut entries, 2, start + Duration::seconds(10)).unwrap();
        for (elapsed, expected) in [(11, 49_000), (59, 1_000)] {
            assert!(matches!(
                check_window(&mut entries, 2, start + Duration::seconds(elapsed)),
                Err(AppError::RateLimited { retry_after_ms }) if retry_after_ms == expected
            ));
            assert_eq!(entries.len(), 2);
        }
        assert!(matches!(
            check_window(
                &mut entries,
                2,
                start + Duration::seconds(60) - Duration::nanoseconds(1)
            ),
            Err(AppError::RateLimited { retry_after_ms: 1 })
        ));
        check_window(&mut entries, 2, start + Duration::seconds(60)).unwrap();
        assert!(matches!(
            check_window(&mut entries, 2, start + Duration::seconds(60)),
            Err(AppError::RateLimited {
                retry_after_ms: 10_000
            })
        ));
    }

    #[test]
    fn database_service_limiter_shares_quota_but_not_between_principals() {
        let limiter = std::sync::Arc::new(crate::RateLimiter::new(1));
        limiter.check("one").unwrap();
        assert!(matches!(limiter.clone().check("one"),
            Err(AppError::RateLimited { retry_after_ms }) if retry_after_ms > 1000 && retry_after_ms <= 60_000));
        limiter.check("two").unwrap();
        assert!(crate::RateLimiter::new(0).check("one").is_err());
    }
}
