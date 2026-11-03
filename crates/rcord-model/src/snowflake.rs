use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Milliseconds between the Unix epoch and the Discord epoch (2015-01-01T00:00:00Z).
pub const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// A Discord ID. Sent as a string on the wire, stored as a non-zero integer, so
/// `Option<Snowflake>` costs nothing extra.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Snowflake(NonZeroU64);

impl Snowflake {
    /// Returns `None` for zero, which Discord never assigns.
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Creation time in milliseconds since the Unix epoch.
    pub const fn timestamp_ms(self) -> u64 {
        (self.get() >> 22) + DISCORD_EPOCH_MS
    }

    /// The smallest ID that could have been created at `unix_ms`, used for pagination.
    pub fn from_timestamp_ms(unix_ms: u64) -> Option<Self> {
        Self::new(unix_ms.saturating_sub(DISCORD_EPOCH_MS) << 22)
    }
}

impl fmt::Debug for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid snowflake")]
pub struct ParseSnowflakeError;

impl FromStr for Snowflake {
    type Err = ParseSnowflakeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u64>()
            .ok()
            .and_then(Self::new)
            .ok_or(ParseSnowflakeError)
    }
}

impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SnowflakeVisitor;

        impl Visitor<'_> for SnowflakeVisitor {
            type Value = Snowflake;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a non-zero snowflake as a string or integer")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Snowflake, E> {
                Snowflake::new(v).ok_or_else(|| E::custom("snowflake is zero"))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Snowflake, E> {
                u64::try_from(v)
                    .ok()
                    .and_then(Snowflake::new)
                    .ok_or_else(|| E::custom("snowflake out of range"))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Snowflake, E> {
                v.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_any(SnowflakeVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strings_and_numbers() {
        let a: Snowflake = serde_json::from_str("\"175928847299117063\"").unwrap();
        let b: Snowflake = serde_json::from_str("175928847299117063").unwrap();
        assert_eq!(a, b);
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"175928847299117063\"");
    }

    #[test]
    fn rejects_zero() {
        assert!(serde_json::from_str::<Snowflake>("\"0\"").is_err());
    }

    #[test]
    fn decodes_timestamp() {
        let id: Snowflake = "175928847299117063".parse().unwrap();
        assert_eq!(id.timestamp_ms(), 1_462_015_105_796);
        let floor = Snowflake::from_timestamp_ms(id.timestamp_ms()).unwrap();
        assert!(floor <= id);
    }

    #[test]
    fn option_is_free() {
        assert_eq!(
            std::mem::size_of::<Option<Snowflake>>(),
            std::mem::size_of::<u64>()
        );
    }
}
