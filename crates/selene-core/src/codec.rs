//! Transient byte-level codec hooks per spec 02 section 7.
//!
//! This trait is for opaque payloads such as extension events and procedure
//! arguments. Durable WAL and snapshot formats use serde/postcard and rkyv
//! projections in their owning crates, not this trait.

use bytes::{Buf, BufMut};

use crate::{BindingTableId, EdgeId, ExtensionTypeId, GraphId, GraphTypeId, NodeId, RecordTypeId};

/// Byte-level encoding trait for transient payloads.
pub trait Codec: Sized {
    /// Error returned by this codec.
    type Error: From<CodecError>;

    /// Encode `self` into `writer`.
    ///
    /// # Errors
    ///
    /// Implementations return their associated error on encoding failure.
    fn encode<B: BufMut>(&self, writer: &mut B) -> Result<(), Self::Error>;

    /// Decode `Self` from `reader`.
    ///
    /// # Errors
    ///
    /// Implementations return their associated error on malformed or truncated
    /// input.
    fn decode<B: Buf>(reader: &mut B) -> Result<Self, Self::Error>;
}

/// Codec failure for transient byte payloads.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The input buffer ended before a complete value could be decoded.
    #[error("buffer too short")]
    BufferTooShort,
    /// A discriminant or sentinel was not valid for the expected type.
    #[error("invalid discriminant for {expected}: {got}")]
    InvalidDiscriminant {
        /// Expected discriminant family.
        expected: &'static str,
        /// Observed discriminant.
        got: u32,
    },
    /// A byte payload was not valid UTF-8.
    #[error("invalid UTF-8")]
    InvalidUtf8,
    /// Bytes remained after decoding a complete value.
    #[error("trailing bytes after decode: {bytes}")]
    Trailing {
        /// Number of remaining bytes.
        bytes: usize,
    },
}

impl CodecError {
    /// Map this error to its implementation-defined GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> &'static str {
        "0G004"
    }
}

fn read_u64<B: Buf>(reader: &mut B) -> Result<u64, CodecError> {
    if reader.remaining() < 8 {
        return Err(CodecError::BufferTooShort);
    }
    Ok(reader.get_u64_le())
}

fn read_u32<B: Buf>(reader: &mut B) -> Result<u32, CodecError> {
    if reader.remaining() < 4 {
        return Err(CodecError::BufferTooShort);
    }
    Ok(reader.get_u32_le())
}

macro_rules! codec_u64_newtype {
    ($Name:ty) => {
        impl Codec for $Name {
            type Error = CodecError;

            fn encode<B: BufMut>(&self, writer: &mut B) -> Result<(), Self::Error> {
                writer.put_u64_le(self.get());
                Ok(())
            }

            fn decode<B: Buf>(reader: &mut B) -> Result<Self, Self::Error> {
                Ok(Self::new(read_u64(reader)?))
            }
        }
    };
}

codec_u64_newtype!(NodeId);
codec_u64_newtype!(EdgeId);
codec_u64_newtype!(GraphId);
codec_u64_newtype!(BindingTableId);
codec_u64_newtype!(RecordTypeId);

impl Codec for GraphTypeId {
    type Error = CodecError;

    fn encode<B: BufMut>(&self, writer: &mut B) -> Result<(), Self::Error> {
        writer.put_u64_le(self.get());
        Ok(())
    }

    fn decode<B: Buf>(reader: &mut B) -> Result<Self, Self::Error> {
        let raw = read_u64(reader)?;
        Self::new(raw).map_err(|_| CodecError::InvalidDiscriminant {
            expected: "non-zero GraphTypeId",
            got: 0,
        })
    }
}

impl Codec for ExtensionTypeId {
    type Error = CodecError;

    fn encode<B: BufMut>(&self, writer: &mut B) -> Result<(), Self::Error> {
        writer.put_u32_le(self.0);
        Ok(())
    }

    fn decode<B: Buf>(reader: &mut B) -> Result<Self, Self::Error> {
        Ok(Self(read_u32(reader)?))
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn node_id_codec_round_trip() {
        let id = NodeId::new(42);
        let mut bytes = BytesMut::new();
        id.encode(&mut bytes).unwrap();
        assert_eq!(NodeId::decode(&mut &bytes[..]).unwrap(), id);
    }

    #[test]
    fn extension_type_id_codec_round_trip() {
        let id = ExtensionTypeId(0x100);
        let mut bytes = BytesMut::new();
        id.encode(&mut bytes).unwrap();
        assert_eq!(ExtensionTypeId::decode(&mut &bytes[..]).unwrap(), id);
    }

    #[test]
    fn codec_error_buffer_too_short_returns() {
        let error = NodeId::decode(&mut &[][..]).unwrap_err();
        assert!(matches!(error, CodecError::BufferTooShort));
    }

    #[test]
    fn codec_error_gqlstatus_is_0g004() {
        let errors = [
            CodecError::BufferTooShort,
            CodecError::InvalidDiscriminant {
                expected: "x",
                got: 7,
            },
            CodecError::InvalidUtf8,
            CodecError::Trailing { bytes: 1 },
        ];
        for error in errors {
            assert_eq!(error.gqlstatus(), "0G004");
        }
    }

    #[test]
    fn multiple_ids_back_to_back_round_trip() {
        let mut bytes = BytesMut::new();
        NodeId::new(1).encode(&mut bytes).unwrap();
        NodeId::new(2).encode(&mut bytes).unwrap();
        let mut reader = &bytes[..];
        assert_eq!(NodeId::decode(&mut reader).unwrap(), NodeId::new(1));
        assert_eq!(NodeId::decode(&mut reader).unwrap(), NodeId::new(2));
    }

    proptest! {
        #[test]
        fn u64_id_codecs_round_trip(raw in any::<u64>()) {
            let mut bytes = BytesMut::new();
            NodeId::new(raw).encode(&mut bytes).unwrap();
            prop_assert_eq!(NodeId::decode(&mut &bytes[..]).unwrap(), NodeId::new(raw));

            bytes.clear();
            EdgeId::new(raw).encode(&mut bytes).unwrap();
            prop_assert_eq!(EdgeId::decode(&mut &bytes[..]).unwrap(), EdgeId::new(raw));

            bytes.clear();
            GraphId::new(raw).encode(&mut bytes).unwrap();
            prop_assert_eq!(GraphId::decode(&mut &bytes[..]).unwrap(), GraphId::new(raw));
        }

        #[test]
        fn graph_type_id_codec_round_trips(raw in 1_u64..u64::MAX) {
            let id = GraphTypeId::new(raw).unwrap();
            let mut bytes = BytesMut::new();
            id.encode(&mut bytes).unwrap();
            prop_assert_eq!(GraphTypeId::decode(&mut &bytes[..]).unwrap(), id);
        }

        #[test]
        fn extension_type_id_codec_round_trips(raw in any::<u32>()) {
            let id = ExtensionTypeId(raw);
            let mut bytes = BytesMut::new();
            id.encode(&mut bytes).unwrap();
            prop_assert_eq!(ExtensionTypeId::decode(&mut &bytes[..]).unwrap(), id);
        }
    }
}
