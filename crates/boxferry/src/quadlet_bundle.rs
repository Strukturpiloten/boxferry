//! Versioned single-file transport for exact native Quadlet bytes. Extract before installation.
use crate::quadlet::{
    QuadletOutput,
    quadlet_lens::model::{QuadletUnitType, UnitFileName},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, error::Error, fmt, io, path::Path};

/// Reserved transport prefix, including unsupported future versions.
pub const RESERVED_MAGIC: &[u8] = b"BOXFERRY-QUADLET-BUNDLE";
/// Exact v1 transport header.
pub const MAGIC: &[u8] = b"BOXFERRY-QUADLET-BUNDLE 1\n";
/// Maximum total serialized transport size.
pub const MAX_BUNDLE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum UTF-8 bytes in one native member.
pub const MAX_MEMBER_BYTES: usize = 8 * 1024 * 1024;
/// Maximum JSON header line size, including its LF.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Maximum number of members in one transport.
pub const MAX_MEMBERS: usize = 4096;

/// One validated native basename and its unchanged UTF-8 contents.
#[derive(Clone, Eq, PartialEq)]
pub struct QuadletBundleMember {
    name: UnitFileName,
    text: String,
}
impl QuadletBundleMember {
    /// Returns the exact native basename.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    /// Returns the native type inferred by the owning Quadlet library.
    #[must_use]
    pub const fn unit_type(&self) -> QuadletUnitType {
        self.name.unit_type()
    }
    /// Explicitly exposes the exact native contents, which may contain protected values.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}
impl fmt::Debug for QuadletBundleMember {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuadletBundleMember")
            .field("name", &self.name)
            .field("text", &"[REDACTED]")
            .field("bytes", &self.text.len())
            .finish()
    }
}
/// Immutable validated transport, preserving member order and every payload byte.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuadletBundle {
    members: Vec<QuadletBundleMember>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
    bytes: u64,
}

/// Value-free transport failure; unsafe names and contents never enter error messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BundleError {
    /// Input is not a supported version of the reserved transport.
    UnsupportedVersion,
    /// Framing, required bytes, JSON fields or canonical decimal spelling are invalid.
    InvalidFraming,
    /// A member basename is unsafe or is not a reviewed native Quadlet unit filename.
    InvalidMemberName,
    /// Exact or ASCII case-insensitive member names collide.
    MemberCollision,
    /// The count, member, header or total size exceeds v1 limits.
    LimitExceeded,
    /// A native payload or header is not valid UTF-8.
    InvalidUtf8,
}
impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedVersion => "unsupported Quadlet bundle transport version",
            Self::InvalidFraming => "invalid Quadlet bundle framing or metadata fields",
            Self::InvalidMemberName => "invalid or unsafe Quadlet bundle member basename",
            Self::MemberCollision => "Quadlet bundle member names collide",
            Self::LimitExceeded => "Quadlet bundle exceeds a version 1 count or size limit",
            Self::InvalidUtf8 => "Quadlet bundle metadata or member contents are not UTF-8",
        })
    }
}
impl Error for BundleError {}
impl QuadletBundle {
    /// Copies an authorized generated native output into a bounded transport.
    ///
    /// # Errors
    /// Rejects limits and unsafe or colliding native names. No content is interpreted or executed.
    pub fn from_output(output: &QuadletOutput) -> Result<Self, BundleError> {
        check_count(output.files().len())?;
        let mut size = preamble_size(output.files().len());
        let mut names = BTreeSet::new();
        let mut members = Vec::with_capacity(output.files().len());
        for file in output.files() {
            let name = validate_name(file.name().as_str(), &mut names)?;
            size = checked_size(size, name.as_str(), file.text().len())?;
            members.push(QuadletBundleMember {
                name,
                text: file.text().to_owned(),
            });
        }
        Ok(Self { members })
    }
    /// Decodes and validates the complete transport before exposing any member.
    ///
    /// # Errors
    /// Rejects malformed/future framing, duplicate/unknown JSON fields, unsafe names, collisions,
    /// invalid UTF-8, excessive lengths, truncated content and trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, BundleError> {
        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err(BundleError::LimitExceeded);
        }
        if !bytes.starts_with(MAGIC) {
            return Err(if bytes.starts_with(RESERVED_MAGIC) {
                BundleError::UnsupportedVersion
            } else {
                BundleError::InvalidFraming
            });
        }
        let mut cursor = MAGIC.len();
        let count_line = line(bytes, &mut cursor, 32)?;
        let count = decimal(
            count_line
                .strip_prefix(b"members ")
                .ok_or(BundleError::InvalidFraming)?,
        )?;
        check_count(count)?;
        let mut members = Vec::with_capacity(count);
        let mut names = BTreeSet::new();
        for _ in 0..count {
            if bytes.get(cursor..cursor.saturating_add(4)) != Some(b"---\n") {
                return Err(BundleError::InvalidFraming);
            }
            cursor += 4;
            let header = line(bytes, &mut cursor, MAX_HEADER_BYTES)?;
            std::str::from_utf8(header).map_err(|_| BundleError::InvalidUtf8)?;
            let header: Header = serde_json::from_slice(header).map_err(|_| BundleError::InvalidFraming)?;
            let name = validate_name(&header.name, &mut names)?;
            let length = usize::try_from(header.bytes).map_err(|_| BundleError::LimitExceeded)?;
            if length > MAX_MEMBER_BYTES {
                return Err(BundleError::LimitExceeded);
            }
            let end = cursor.checked_add(length).ok_or(BundleError::LimitExceeded)?;
            let text = std::str::from_utf8(bytes.get(cursor..end).ok_or(BundleError::InvalidFraming)?)
                .map_err(|_| BundleError::InvalidUtf8)?;
            if bytes.get(end) != Some(&b'\n') {
                return Err(BundleError::InvalidFraming);
            }
            cursor = end + 1;
            members.push(QuadletBundleMember {
                name,
                text: text.to_owned(),
            });
        }
        if cursor != bytes.len() {
            return Err(BundleError::InvalidFraming);
        }
        Ok(Self { members })
    }
    /// Encodes canonical v1 framing, without changing member order or native bytes.
    ///
    /// # Errors
    /// Rejects encoded size limits or a header serialization failure.
    pub fn encode(&self) -> Result<Vec<u8>, BundleError> {
        let mut size = preamble_size(self.members.len());
        for member in &self.members {
            size = checked_size(size, member.name(), member.text.len())?;
        }
        let mut output = Vec::with_capacity(size);
        output.extend_from_slice(MAGIC);
        output.extend_from_slice(format!("members {}\n", self.members.len()).as_bytes());
        for member in &self.members {
            output.extend_from_slice(b"---\n");
            output.extend_from_slice(&header(member.name(), member.text.len())?);
            output.push(b'\n');
            output.extend_from_slice(member.text.as_bytes());
            output.push(b'\n');
        }
        Ok(output)
    }
    /// Returns members in their original order.
    #[must_use]
    pub fn members(&self) -> &[QuadletBundleMember] {
        &self.members
    }
    /// Extracts unchanged native bytes using private create-new files and owned cleanup.
    ///
    /// This is transport extraction, not semantic conversion or native installation.
    ///
    /// # Errors
    /// Rejects an invalid destination or any I/O failure before/while writing. Existing files
    /// are never overwritten; every transport byte was validated before directory creation.
    pub fn extract_to(&self, directory: &Path) -> io::Result<()> {
        let files = self
            .members
            .iter()
            .map(|member| crate::output::OutputFile::new(member.name(), member.text.as_bytes()))
            .collect::<Vec<_>>();
        crate::output::write_new_files(directory, &files, true)
    }
}
fn validate_name(name: &str, names: &mut BTreeSet<String>) -> Result<UnitFileName, BundleError> {
    if !crate::output::safe_basename(name) {
        return Err(BundleError::InvalidMemberName);
    }
    let name = UnitFileName::new(name).map_err(|_| BundleError::InvalidMemberName)?;
    if !names.insert(name.as_str().to_ascii_lowercase()) {
        return Err(BundleError::MemberCollision);
    }
    Ok(name)
}
fn line<'a>(bytes: &'a [u8], cursor: &mut usize, maximum: usize) -> Result<&'a [u8], BundleError> {
    let rest = bytes.get(*cursor..).ok_or(BundleError::InvalidFraming)?;
    let search = &rest[..rest.len().min(maximum)];
    let end = search
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or(if rest.len() >= maximum {
            BundleError::LimitExceeded
        } else {
            BundleError::InvalidFraming
        })?;
    *cursor += end + 1;
    Ok(&rest[..end])
}
fn decimal(bytes: &[u8]) -> Result<usize, BundleError> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) || (bytes.len() > 1 && bytes[0] == b'0') {
        return Err(BundleError::InvalidFraming);
    }
    std::str::from_utf8(bytes)
        .map_err(|_| BundleError::InvalidUtf8)?
        .parse()
        .map_err(|_| BundleError::LimitExceeded)
}
fn check_count(count: usize) -> Result<(), BundleError> {
    if (1..=MAX_MEMBERS).contains(&count) {
        Ok(())
    } else {
        Err(BundleError::LimitExceeded)
    }
}
fn preamble_size(count: usize) -> usize {
    MAGIC.len() + format!("members {count}\n").len()
}
fn header(name: &str, length: usize) -> Result<Vec<u8>, BundleError> {
    let bytes = u64::try_from(length).map_err(|_| BundleError::LimitExceeded)?;
    let text = serde_json::to_vec(&Header {
        name: name.to_owned(),
        bytes,
    })
    .map_err(|_| BundleError::InvalidFraming)?;
    if text.len() >= MAX_HEADER_BYTES {
        return Err(BundleError::LimitExceeded);
    }
    Ok(text)
}
fn checked_size(size: usize, name: &str, length: usize) -> Result<usize, BundleError> {
    if length > MAX_MEMBER_BYTES {
        return Err(BundleError::LimitExceeded);
    }
    let size = size
        .checked_add(header(name, length)?.len())
        .and_then(|size| size.checked_add(length))
        .and_then(|size| size.checked_add(6))
        .ok_or(BundleError::LimitExceeded)?;
    if size > MAX_BUNDLE_BYTES {
        Err(BundleError::LimitExceeded)
    } else {
        Ok(size)
    }
}
