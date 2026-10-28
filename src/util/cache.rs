//! Limits shared by in-process caches built on moka.

use std::time::Duration;

/// Longest idle expiry moka accepts; it panics on longer durations.
pub const MAX_TIME_TO_IDLE: Duration = Duration::from_secs(1_000 * 365 * 24 * 3600);

/// A `time_to_idle` that is safe to pass to moka. Values beyond
/// [`MAX_TIME_TO_IDLE`] disable idle expiry (with a warning) instead of
/// panicking, since such a setting means "keep entries" in practice.
pub fn checked_time_to_idle(time_to_idle: Option<Duration>) -> Option<Duration> {
    time_to_idle.filter(|tti| {
        let accepted = *tti <= MAX_TIME_TO_IDLE;
        if !accepted {
            tracing::warn!(
                "cache time_to_idle {}s exceeds {}s; idle expiry is disabled",
                tti.as_secs(),
                MAX_TIME_TO_IDLE.as_secs()
            );
        }
        accepted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_values_up_to_the_limit() {
        assert_eq!(checked_time_to_idle(None), None);
        let one = Some(Duration::from_secs(1));
        assert_eq!(checked_time_to_idle(one), one);
        assert_eq!(
            checked_time_to_idle(Some(MAX_TIME_TO_IDLE)),
            Some(MAX_TIME_TO_IDLE)
        );
    }

    #[test]
    fn disables_idle_expiry_beyond_the_limit() {
        let over = MAX_TIME_TO_IDLE + Duration::from_secs(1);
        assert_eq!(checked_time_to_idle(Some(over)), None);
        assert_eq!(checked_time_to_idle(Some(Duration::MAX)), None);
    }

    #[test]
    fn limit_matches_what_moka_accepts() {
        let _cache: moka::sync::Cache<u8, u8> = moka::sync::Cache::builder()
            .time_to_idle(MAX_TIME_TO_IDLE)
            .build();
    }
}
