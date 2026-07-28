use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IdParseError {
    pub message: String,
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for IdParseError {}

fn validate_id(name: &'static str, value: &str) -> Result<(), IdParseError> {
    if value.is_empty() || value.len() > 64 {
        return Err(IdParseError {
            message: format!("{} must be 1-64 characters, got {}", name, value.len()),
        });
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-')
    {
        return Err(IdParseError {
            message: format!(
                "{} contains invalid characters: only lowercase ASCII letters, digits, '.', '_', '-' are allowed",
                name
            ),
        });
    }
    Ok(())
}

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl AsRef<str>) -> Result<Self, IdParseError> {
                let s = value.as_ref();
                validate_id(stringify!($name), s)?;
                Ok(Self(s.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }
    };
}

define_id!(TargetId);
define_id!(CheckId);
define_id!(ProfileId);
define_id!(RuleId);
define_id!(NotifierId);
define_id!(ObserverId);
define_id!(CheckRevisionId);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ConfigGeneration(String);

impl ConfigGeneration {
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(value.as_ref().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConfigGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RoundId(pub i64);

impl RoundId {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn value(self) -> i64 {
        self.0
    }
}

impl fmt::Display for RoundId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_target_id() {
        let id = TargetId::new("my-target_1.0").unwrap();
        assert_eq!(id.as_str(), "my-target_1.0");
    }

    #[test]
    fn reject_empty_id() {
        assert!(TargetId::new("").is_err());
    }

    #[test]
    fn reject_too_long_id() {
        let long = "a".repeat(65);
        assert!(TargetId::new(&long).is_err());
    }

    #[test]
    fn reject_uppercase_id() {
        assert!(TargetId::new("MyTarget").is_err());
    }

    #[test]
    fn reject_special_chars_id() {
        assert!(TargetId::new("my@target").is_err());
    }

    #[test]
    fn max_length_id() {
        let s = "a".repeat(64);
        assert!(TargetId::new(&s).is_ok());
    }

    #[test]
    fn from_str_roundtrip() {
        let id: TargetId = "hello-world".parse().unwrap();
        assert_eq!(id.to_string(), "hello-world");
    }
}
