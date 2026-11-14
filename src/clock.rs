use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, Duration, OffsetDateTime, UtcOffset};

use crate::error::{Error, Result};

pub fn now_from_env(value: Option<&str>) -> Result<OffsetDateTime> {
    match value {
        Some(raw) if !raw.trim().is_empty() => parse_timestamp(raw.trim())
            .map_err(|_| Error::usage(format!("FLIS_NOW is not an RFC 3339 timestamp: {raw}"))),
        _ => Ok(OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap_or_else(|_| OffsetDateTime::now_utc())),
    }
}

pub fn parse_timestamp(raw: &str) -> std::result::Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(raw, &Rfc3339).map(|at| at.to_offset(UtcOffset::UTC))
}

pub fn format_timestamp(at: OffsetDateTime) -> String {
    let format = format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    at.to_offset(UtcOffset::UTC)
        .format(&format)
        .unwrap_or_else(|_| at.unix_timestamp().to_string())
}

pub fn date_part(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

/// Accepts `90d`, `2w`, `36h`, a date (`2026-08-01`) or an RFC 3339 timestamp.
pub fn parse_since(raw: &str, now: OffsetDateTime) -> Result<OffsetDateTime> {
    let raw = raw.trim();
    let invalid = || {
        Error::usage(format!(
            "--since {raw:?} is not a duration (90d, 2w, 36h), a date (2026-08-01) or a timestamp"
        ))
    };
    if let Some(unit) = raw.chars().last().filter(|c| c.is_ascii_alphabetic())
        && let Ok(count) = raw[..raw.len() - 1].parse::<i64>()
    {
        let span = match unit {
            'd' => Duration::days(count),
            'w' => Duration::weeks(count),
            'h' => Duration::hours(count),
            _ => return Err(invalid()),
        };
        return Ok(now - span);
    }
    if let Ok(at) = parse_timestamp(raw) {
        return Ok(at);
    }
    Date::parse(raw, format_description!("[year]-[month]-[day]"))
        .map(|date| date.midnight().assume_utc())
        .map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn formats_utc_without_fraction() {
        let at = datetime!(2026-11-14 21:22:05.123 +01:00);
        assert_eq!(format_timestamp(at), "2026-11-14T20:22:05Z");
    }

    #[test]
    fn parses_since_forms() {
        let now = datetime!(2026-11-14 12:00 UTC);
        assert_eq!(
            parse_since("90d", now).unwrap(),
            datetime!(2026-08-16 12:00 UTC)
        );
        assert_eq!(
            parse_since("2w", now).unwrap(),
            datetime!(2026-10-31 12:00 UTC)
        );
        assert_eq!(
            parse_since("6h", now).unwrap(),
            datetime!(2026-11-14 06:00 UTC)
        );
        assert_eq!(
            parse_since("2026-10-01", now).unwrap(),
            datetime!(2026-10-01 0:00 UTC)
        );
        assert_eq!(
            parse_since("2026-10-01T10:00:00+02:00", now).unwrap(),
            datetime!(2026-10-01 8:00 UTC)
        );
        assert!(parse_since("soon", now).is_err());
        assert!(parse_since("3m", now).is_err());
    }

    #[test]
    fn flis_now_overrides_the_clock() {
        let at = now_from_env(Some("2026-01-02T03:04:05Z")).unwrap();
        assert_eq!(format_timestamp(at), "2026-01-02T03:04:05Z");
        assert!(now_from_env(Some("yesterday")).is_err());
    }
}
