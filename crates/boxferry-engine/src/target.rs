//! Target implementation version ranges.

use std::{error::Error, fmt};

/// Numeric target implementation version.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlatformVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl PlatformVersion {
    /// Creates a numeric version.
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self { major, minor, patch }
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

    /// Returns the patch number.
    #[must_use]
    pub const fn patch(self) -> u64 {
        self.patch
    }
}

impl fmt::Display for PlatformVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Inclusive minimum and optional maximum target versions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRange {
    minimum: PlatformVersion,
    maximum: Option<PlatformVersion>,
}

impl VersionRange {
    /// Creates an inclusive version range.
    ///
    /// # Errors
    ///
    /// Returns [`TargetProfileError::MaximumBeforeMinimum`] for an inverted range.
    pub const fn new(minimum: PlatformVersion, maximum: Option<PlatformVersion>) -> Result<Self, TargetProfileError> {
        if let Some(maximum) = maximum {
            if version_is_before(maximum, minimum) {
                return Err(TargetProfileError::MaximumBeforeMinimum { minimum, maximum });
            }
        }
        Ok(Self { minimum, maximum })
    }

    /// Returns the inclusive minimum version.
    #[must_use]
    pub const fn minimum(self) -> PlatformVersion {
        self.minimum
    }

    /// Returns the inclusive optional maximum version.
    #[must_use]
    pub const fn maximum(self) -> Option<PlatformVersion> {
        self.maximum
    }

    /// Returns whether a version is inside the inclusive range.
    #[must_use]
    pub const fn contains(self, version: PlatformVersion) -> bool {
        !version_is_before(version, self.minimum)
            && match self.maximum {
                Some(maximum) => !version_is_before(maximum, version),
                None => true,
            }
    }
}

/// Invalid target profile.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TargetProfileError {
    /// The target implementation name was empty or contained a NUL byte.
    InvalidImplementation,
    /// The optional maximum version was before the minimum.
    MaximumBeforeMinimum {
        /// Inclusive minimum.
        minimum: PlatformVersion,
        /// Invalid inclusive maximum.
        maximum: PlatformVersion,
    },
}

impl fmt::Display for TargetProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidImplementation => {
                formatter.write_str("target implementation must be non-empty and contain no NUL byte")
            }
            Self::MaximumBeforeMinimum { minimum, maximum } => {
                write!(formatter, "target maximum {maximum} is before minimum {minimum}")
            }
        }
    }
}

impl Error for TargetProfileError {}

/// Caller-selected target implementation and supported compatibility range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetProfile {
    implementation: String,
    versions: VersionRange,
}

impl TargetProfile {
    /// Creates a target profile without interpreting implementation-specific capabilities.
    ///
    /// # Errors
    ///
    /// Returns [`TargetProfileError`] for an invalid name or version range.
    pub fn new(
        implementation: impl Into<String>,
        minimum_version: PlatformVersion,
        maximum_version: Option<PlatformVersion>,
    ) -> Result<Self, TargetProfileError> {
        let implementation = implementation.into();
        if implementation.is_empty() || implementation.contains('\0') {
            return Err(TargetProfileError::InvalidImplementation);
        }
        Ok(Self {
            implementation,
            versions: VersionRange::new(minimum_version, maximum_version)?,
        })
    }

    /// Returns the target implementation name, such as `podman`.
    #[must_use]
    pub fn implementation(&self) -> &str {
        &self.implementation
    }

    /// Returns the requested compatibility range.
    #[must_use]
    pub const fn versions(&self) -> VersionRange {
        self.versions
    }
}

const fn version_is_before(left: PlatformVersion, right: PlatformVersion) -> bool {
    left.major < right.major
        || (left.major == right.major && left.minor < right.minor)
        || (left.major == right.major && left.minor == right.minor && left.patch < right.patch)
}

#[cfg(test)]
mod tests {
    use super::{PlatformVersion, TargetProfile, TargetProfileError};

    #[test]
    fn minimum_and_maximum_are_inclusive() -> Result<(), String> {
        let profile =
            TargetProfile::new("podman", version(5, 4), Some(version(5, 6))).map_err(|error| error.to_string())?;
        assert!(profile.versions().contains(version(5, 4)));
        assert!(profile.versions().contains(version(5, 6)));
        assert!(!profile.versions().contains(version(5, 3)));
        assert!(!profile.versions().contains(version(5, 7)));
        Ok(())
    }

    #[test]
    fn rejects_maximum_before_minimum() {
        assert!(matches!(
            TargetProfile::new("podman", version(5, 4), Some(version(5, 3))),
            Err(TargetProfileError::MaximumBeforeMinimum { .. })
        ));
    }

    const fn version(major: u64, minor: u64) -> PlatformVersion {
        PlatformVersion::new(major, minor, 0)
    }
}
