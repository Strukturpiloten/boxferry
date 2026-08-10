//! Tolerant real-world container image references.

use crate::ModelError;

/// A preserved image reference with optional tag and digest components.
///
/// Unlike a strict OCI-only parser, this type deliberately accepts the common
/// `name:tag@algorithm:digest` form used by Docker Compose and Podman.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageReference {
    authored: String,
    repository: String,
    tag: Option<String>,
    digest: Option<String>,
}

impl ImageReference {
    /// Parses a preserved image reference without normalizing its spelling.
    ///
    /// # Errors
    ///
    /// Returns a [`ModelError`] for empty components, multiple digest
    /// separators, or embedded NUL bytes.
    pub fn parse(authored: impl Into<String>) -> Result<Self, ModelError> {
        let authored = authored.into();
        validate_component("image reference", &authored)?;

        let mut digest_parts = authored.split('@');
        let name_and_tag = digest_parts.next().unwrap_or_default();
        let digest = digest_parts.next();
        if digest_parts.next().is_some() {
            return Err(ModelError::InvalidImageReference(
                "multiple `@` digest separators are not supported",
            ));
        }
        if let Some(value) = digest {
            validate_component("image digest", value)?;
        }

        let last_slash = name_and_tag.rfind('/');
        let last_colon = name_and_tag.rfind(':');
        let has_tag = last_colon.is_some_and(|colon| last_slash.is_none_or(|slash| colon > slash));
        let (repository, tag) = if has_tag {
            let colon = last_colon.unwrap_or_default();
            (&name_and_tag[..colon], Some(&name_and_tag[colon + 1..]))
        } else {
            (name_and_tag, None)
        };
        validate_component("image repository", repository)?;
        if let Some(value) = tag {
            validate_component("image tag", value)?;
        }

        let repository = repository.to_owned();
        let tag = tag.map(str::to_owned);
        let digest = digest.map(str::to_owned);
        Ok(Self {
            authored,
            repository,
            tag,
            digest,
        })
    }

    /// Returns the complete authored spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.authored
    }

    /// Returns the repository/name component without tag or digest.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Returns the optional tag without its colon.
    #[must_use]
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// Returns the optional digest without its `@` separator.
    #[must_use]
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
}

fn validate_component(kind: &'static str, value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::EmptyValue(kind));
    }
    if value.contains('\0') {
        return Err(ModelError::ContainsNul(kind));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ImageReference;

    #[test]
    fn accepts_registry_port_tag_and_digest_together() -> Result<(), String> {
        let image = ImageReference::parse("registry.example:5000/team/app:1.2@sha256:abcd")
            .map_err(|error| error.to_string())?;
        assert_eq!(image.repository(), "registry.example:5000/team/app");
        assert_eq!(image.tag(), Some("1.2"));
        assert_eq!(image.digest(), Some("sha256:abcd"));
        assert_eq!(image.as_str(), "registry.example:5000/team/app:1.2@sha256:abcd");
        Ok(())
    }

    #[test]
    fn does_not_treat_a_registry_port_as_a_tag() -> Result<(), String> {
        let image =
            ImageReference::parse("registry.example:5000/team/app@sha256:abcd").map_err(|error| error.to_string())?;
        assert_eq!(image.repository(), "registry.example:5000/team/app");
        assert_eq!(image.tag(), None);
        Ok(())
    }
}
