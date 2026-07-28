use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PercentageParseError {
    #[error("empty percentage string")]
    Empty,
    #[error("percentage must end with '%': {0}")]
    MissingPercent(String),
    #[error("invalid number in percentage: {0}")]
    InvalidNumber(String),
    #[error("percentage must be 0-100, got {0}")]
    OutOfRange(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Percentage(u8);

impl Percentage {
    pub fn new(value: u8) -> Result<Self, PercentageParseError> {
        if value > 100 {
            return Err(PercentageParseError::OutOfRange(value));
        }
        Ok(Self(value))
    }

    pub fn value(self) -> u8 {
        self.0
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64 / 100.0
    }
}

impl FromStr for Percentage {
    type Err = PercentageParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PercentageParseError::Empty);
        }
        let s = s
            .strip_suffix('%')
            .ok_or_else(|| PercentageParseError::MissingPercent(s.to_owned()))?;
        let value: u8 = s
            .parse()
            .map_err(|_| PercentageParseError::InvalidNumber(s.to_owned()))?;
        Self::new(value)
    }
}

impl fmt::Display for Percentage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_percentage() {
        let p: Percentage = "10%".parse().unwrap();
        assert_eq!(p.value(), 10);
    }

    #[test]
    fn zero_percentage() {
        let p: Percentage = "0%".parse().unwrap();
        assert_eq!(p.value(), 0);
    }

    #[test]
    fn hundred_percentage() {
        let p: Percentage = "100%".parse().unwrap();
        assert_eq!(p.value(), 100);
    }

    #[test]
    fn over_hundred() {
        assert!(Percentage::new(101).is_err());
        assert!("101%".parse::<Percentage>().is_err());
    }

    #[test]
    fn missing_percent_sign() {
        assert!("50".parse::<Percentage>().is_err());
    }

    #[test]
    fn display() {
        let p = Percentage::new(75).unwrap();
        assert_eq!(format!("{}", p), "75%");
    }

    #[test]
    fn as_f64() {
        let p = Percentage::new(10).unwrap();
        assert!((p.as_f64() - 0.1).abs() < f64::EPSILON);
    }
}
