use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DurationParseError {
    #[error("empty duration string")]
    Empty,
    #[error("no unit suffix in duration: {0}")]
    NoUnit(String),
    #[error("invalid number in duration: {0}")]
    InvalidNumber(String),
    #[error("unknown unit in duration: {0}")]
    UnknownUnit(String),
    #[error("duration value would overflow: {0}")]
    Overflow(String),
}

pub fn parse_duration(s: &str) -> Result<Duration, DurationParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(DurationParseError::Empty);
    }

    let (num_part, unit) = split_duration(s)?;
    let value: u64 = num_part
        .parse()
        .map_err(|_| DurationParseError::InvalidNumber(num_part.to_owned()))?;

    let millis = match unit {
        "ms" => Some(value),
        "s" => value.checked_mul(1_000),
        "m" => value.checked_mul(60_000),
        "h" => value.checked_mul(3_600_000),
        "d" => value.checked_mul(86_400_000),
        _ => return Err(DurationParseError::UnknownUnit(unit.to_owned())),
    };

    match millis {
        Some(m) => Ok(Duration::from_millis(m)),
        None => Err(DurationParseError::Overflow(s.to_owned())),
    }
}

fn split_duration(s: &str) -> Result<(&str, &str), DurationParseError> {
    let idx = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i);

    match idx {
        None => Err(DurationParseError::NoUnit(s.to_owned())),
        Some(i) => {
            if i == 0 {
                return Err(DurationParseError::NoUnit(s.to_owned()));
            }
            Ok((&s[..i], &s[i..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliseconds() {
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
    }

    #[test]
    fn seconds() {
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn minutes() {
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn hours() {
        assert_eq!(parse_duration("24h").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn days() {
        assert_eq!(
            parse_duration("14d").unwrap(),
            Duration::from_secs(14 * 86400)
        );
    }

    #[test]
    fn empty_string() {
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn no_unit() {
        assert!(parse_duration("100").is_err());
    }

    #[test]
    fn unknown_unit() {
        assert!(parse_duration("5w").is_err());
    }

    #[test]
    fn invalid_number() {
        assert!(parse_duration("abcs").is_err());
    }
}
