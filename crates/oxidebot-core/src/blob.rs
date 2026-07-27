use crate::RetainedSize;
use bytes::Bytes;
use std::ops::Deref;
use thiserror::Error;

/// Error returned when a shared byte view is charged for fewer bytes than it exposes.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("retained-byte charge must be at least the visible byte length")]
pub struct InvalidRetainedByteCharge;

/// Shared immutable bytes with an explicit retained-memory charge.
///
/// A `Bytes` slice can keep a much larger backing allocation alive than its
/// visible `len()`. Queue admission therefore uses `charged_bytes` rather than
/// assuming that the visible length is the complete retained allocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedBytes {
    bytes: Bytes,
    charged_bytes: usize,
}

impl RetainedBytes {
    /// Copies the visible byte range into a compact owned allocation.
    ///
    /// Copying is deliberate: an arbitrary `Bytes` value may be a tiny slice of
    /// a much larger shared allocation, whose retained size cannot be discovered
    /// from the view. Use [`Self::with_charge`] to preserve zero-copy sharing when
    /// the backing-allocation charge is known by the adapter.
    #[must_use]
    pub fn new(bytes: Bytes) -> Self {
        let compact = Bytes::copy_from_slice(bytes.as_ref());
        let charged_bytes = compact.len();
        Self {
            bytes: compact,
            charged_bytes,
        }
    }

    /// Wraps shared bytes with the known backing-allocation charge.
    pub fn with_charge(
        bytes: Bytes,
        charged_bytes: usize,
    ) -> Result<Self, InvalidRetainedByteCharge> {
        if charged_bytes < bytes.len() {
            return Err(InvalidRetainedByteCharge);
        }
        Ok(Self {
            bytes,
            charged_bytes,
        })
    }

    /// Converts an owned vector while charging its allocated capacity.
    #[must_use]
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        let charged_bytes = bytes.capacity();
        Self {
            bytes: Bytes::from(bytes),
            charged_bytes,
        }
    }

    /// Returns the visible immutable bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Returns the visible byte length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the visible byte view is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns the charge used by queue admission.
    #[must_use]
    pub const fn charged_bytes(&self) -> usize {
        self.charged_bytes
    }

    /// Consumes the wrapper and returns the shared byte view.
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        self.bytes
    }
}

impl From<Vec<u8>> for RetainedBytes {
    fn from(value: Vec<u8>) -> Self {
        Self::from_vec(value)
    }
}

impl Deref for RetainedBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.bytes.as_ref()
    }
}

impl RetainedSize for RetainedBytes {
    fn retained_bytes(&self) -> usize {
        self.charged_bytes
            .saturating_add(std::mem::size_of::<Self>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_charge_cannot_understate_visible_bytes() {
        assert_eq!(
            RetainedBytes::with_charge(Bytes::from_static(b"abcd"), 3),
            Err(InvalidRetainedByteCharge)
        );
    }

    #[test]
    fn arbitrary_bytes_are_compacted_to_the_visible_range() {
        let backing = Bytes::from(vec![7_u8; 4096]);
        let visible = backing.slice(0..4);
        let retained = RetainedBytes::new(visible);
        assert_eq!(retained.len(), 4);
        assert_eq!(retained.charged_bytes(), 4);
    }

    #[test]
    fn owned_vectors_charge_their_capacity() {
        let mut value = Vec::with_capacity(64);
        value.extend_from_slice(b"abc");
        let bytes = RetainedBytes::from_vec(value);
        assert_eq!(bytes.len(), 3);
        assert!(bytes.charged_bytes() >= 64);
    }
}
