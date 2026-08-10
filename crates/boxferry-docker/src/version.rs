//! Docker Engine API version values.

use std::{error::Error, fmt, str::FromStr};

/// Error returned when an Engine API version is not exactly `major.minor`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseDockerApiVersionError;

impl fmt::Display for ParseDockerApiVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Docker Engine API version must contain exactly two unsigned numbers: major.minor")
    }
}

impl Error for ParseDockerApiVersionError {}

/// Numeric Docker Engine API version, such as `1.40` or `1.55`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DockerApiVersion {
    major: u64,
    minor: u64,
}

impl DockerApiVersion {
    /// Creates an Engine API version.
    #[must_use]
    pub const fn new(major: u64, minor: u64) -> Self {
        Self { major, minor }
    }

    /// Returns the major number.
    #[must_use]
    pub const fn major(self) -> u64 {
        self.major
    }

    /// Returns the minor number.
    #[must_use]
    pub const fn minor(self) -> u64 {
        self.minor
    }
}

impl fmt::Display for DockerApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for DockerApiVersion {
    type Err = ParseDockerApiVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut components = value.split('.');
        let major = parse_component(components.next())?;
        let minor = parse_component(components.next())?;
        if components.next().is_some() {
            return Err(ParseDockerApiVersionError);
        }
        Ok(Self::new(major, minor))
    }
}

fn parse_component(value: Option<&str>) -> Result<u64, ParseDockerApiVersionError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or(ParseDockerApiVersionError)?
        .parse()
        .map_err(|_| ParseDockerApiVersionError)
}

#[cfg(test)]
mod tests {
    use super::DockerApiVersion;

    #[test]
    fn parses_exact_two_component_versions() -> Result<(), String> {
        assert_eq!(
            "1.55".parse::<DockerApiVersion>().map_err(|error| error.to_string())?,
            DockerApiVersion::new(1, 55)
        );
        for value in ["", "1", "1.55.0", "v1.55", "1.-1"] {
            assert!(value.parse::<DockerApiVersion>().is_err(), "{value} must fail");
        }
        Ok(())
    }
}
