use choruz_common::AppError;
use chrono::{DateTime, Duration, Utc};

/// Calculate the next scheduled instant strictly after `after`.
/// Cron uses the supplied IANA timezone (UTC when absent); intervals are elapsed
/// durations. Invalid expressions, zones and non-positive intervals are validation errors.
pub fn next_run_at(
    schedule_type: &str,
    value: &str,
    timezone: Option<&str>,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    let invalid = |detail: &str| AppError::Validation(format!("invalid schedule: {detail}"));
    let zone = timezone
        .unwrap_or("UTC")
        .parse::<chrono_tz::Tz>()
        .map_err(|_| invalid("timezone must be an IANA timezone"))?;
    let next = match schedule_type {
        "cron" => value
            .parse::<croner::Cron>()
            .map_err(|error| invalid(&error.to_string()))?
            .find_next_occurrence(&after.with_timezone(&zone), false)
            .map_err(|error| invalid(&error.to_string()))?
            .with_timezone(&Utc),
        "every" => {
            let value = value.trim();
            let (number, multiplier) = if let Some(number) = value.strip_suffix('s') {
                (number, 1)
            } else if let Some(number) = value.strip_suffix('m') {
                (number, 60)
            } else if let Some(number) = value.strip_suffix('h') {
                (number, 3600)
            } else if let Some(number) = value.strip_suffix('d') {
                (number, 86400)
            } else {
                (value, 60)
            };
            let seconds = number
                .parse::<i64>()
                .ok()
                .and_then(|n| n.checked_mul(multiplier))
                .filter(|n| *n > 0)
                .ok_or_else(|| {
                    invalid("interval must be positive seconds, minutes, hours or days")
                })?;
            let duration =
                Duration::try_seconds(seconds).ok_or_else(|| invalid("interval is too large"))?;
            after
                .checked_add_signed(duration)
                .ok_or_else(|| invalid("interval is too large"))?
        }
        "at" => value
            .parse::<DateTime<Utc>>()
            .map_err(|_| invalid("timestamp must include its UTC offset"))?,
        _ => return Err(invalid("type must be at, every or cron")),
    };
    if next <= after {
        return Err(invalid("next occurrence must be in the future"));
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_matches_calendar_and_timezone_not_creation_delay() {
        let now = "2026-09-05T08:07:31Z".parse().unwrap();
        for (pattern, zone, expected) in [
            ("*/15 * * * *", "UTC", "2026-09-05T08:15:00Z"),
            ("0 10 * * *", "Asia/Shanghai", "2026-09-06T02:00:00Z"),
            ("0 10 * * MON", "UTC", "2026-09-07T10:00:00Z"),
        ] {
            assert_eq!(
                next_run_at("cron", pattern, Some(zone), now).unwrap(),
                expected.parse::<DateTime<Utc>>().unwrap()
            );
        }
    }

    #[test]
    fn intervals_and_one_shots_are_exact_and_invalid_values_are_rejected() {
        let now = "2026-09-05T08:07:31Z".parse().unwrap();
        assert_eq!(
            next_run_at("every", "15m", None, now).unwrap(),
            now + Duration::minutes(15)
        );
        assert_eq!(
            next_run_at("every", "15", None, now).unwrap(),
            now + Duration::minutes(15)
        );
        for (value, seconds) in [("30s", 30), (" 10m ", 600), ("2h", 7200), ("7d", 604800)] {
            assert_eq!(
                next_run_at("every", value, None, now).unwrap(),
                now + Duration::seconds(seconds)
            );
        }
        for value in ["0s", "-5m", "9223372036854775807d", "é", ""] {
            assert!(next_run_at("every", value, None, now).is_err());
        }
        assert!(next_run_at("cron", "not cron", None, now).is_err());
        assert!(next_run_at("cron", "* * * * *", Some("wrong-zone"), now).is_err());
        assert!(next_run_at("at", "2026-09-05T08:07:31Z", None, now).is_err());
        assert_eq!(
            next_run_at("at", "2026-09-06T10:00:00+08:00", None, now).unwrap(),
            "2026-09-06T02:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn daily_wall_time_tracks_daylight_saving_offset() {
        let before = "2026-03-07T15:00:00Z".parse().unwrap();
        assert_eq!(
            next_run_at("cron", "0 9 * * *", Some("America/New_York"), before).unwrap(),
            "2026-03-08T13:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
