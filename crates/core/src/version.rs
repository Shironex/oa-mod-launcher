//! Version numbers of the mod and of the launcher.
//!
//! A version is exactly three numbers, `major.minor.patch`. The text of a
//! version becomes part of a file name, so nothing looser is accepted: no `v`
//! prefix, no suffix, no empty part, no sign.

use std::fmt;
use std::str::FromStr;

/// A parsed `major.minor.patch` version. Ordered the way the numbers read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// First number.
    pub major: u32,
    /// Second number.
    pub minor: u32,
    /// Third number.
    pub patch: u32,
}

impl Version {
    /// Parse `text`, or `None` when it is not three plain numbers.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let major = number(parts.next()?)?;
        let minor = number(parts.next()?)?;
        let patch = number(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

/// One part of a version: digits only, at most nine of them, so "+1", " 1" and
/// a number that overflows are all refused.
fn number(part: &str) -> Option<u32> {
    if part.is_empty() || part.len() > 9 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

impl FromStr for Version {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text).ok_or(())
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Whether `text` is a version this launcher accepts.
pub fn is_valid(text: &str) -> bool {
    Version::parse(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_plain_numbers() {
        assert_eq!(
            Version::parse("0.9.1"),
            Some(Version {
                major: 0,
                minor: 9,
                patch: 1
            })
        );
        assert_eq!(
            Version::parse("12.0.340").map(|v| v.to_string()),
            Some("12.0.340".to_owned())
        );
    }

    #[test]
    fn refuses_everything_else() {
        for text in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "1.2.3-rc.1",
            "1..3",
            "1.2.",
            " 1.2.3",
            "1.2.+3",
            "1.2.x",
            "..",
            "../..",
            "1.2.3/..",
            "1.2.99999999999",
        ] {
            assert_eq!(Version::parse(text), None, "{text:?} must not parse");
        }
    }

    #[test]
    fn orders_by_number_not_by_text() {
        let parse = |text| Version::parse(text).expect("a valid version");
        assert!(parse("0.10.0") > parse("0.9.9"));
        assert!(parse("1.0.0") > parse("0.99.99"));
        assert!(parse("0.9.1") > parse("0.9.0"));
        assert_eq!(parse("0.9.0"), parse("0.9.0"));
    }
}
