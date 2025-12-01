//! Snapshot codec abstractions for serializing/deserializing KV state.
//!
//! This module provides a trait for snapshot serialization and implementations
//! for different formats. Using a trait allows:
//! - Testing with simpler formats (JSON for debugging)
//! - Changing serialization without modifying Node
//! - Separation of concerns (state machine vs serialization)

use std::collections::BTreeMap;
use thiserror::Error;

/// Errors that can occur during snapshot serialization/deserialization.
#[derive(Debug, Error)]
pub enum SnapshotError {
    /// Failed to encode snapshot data.
    #[error("Failed to encode snapshot: {0}")]
    EncodeError(String),

    /// Failed to decode snapshot data.
    #[error("Failed to decode snapshot: {0}")]
    DecodeError(String),
}

/// Trait for encoding and decoding KV snapshots.
///
/// Implementations provide different serialization formats for the KV state.
pub trait SnapshotCodec {
    /// Encode a KV map to bytes.
    fn encode(&self, data: &BTreeMap<String, String>) -> Result<Vec<u8>, SnapshotError>;

    /// Decode bytes to a KV map.
    fn decode(&self, bytes: &[u8]) -> Result<BTreeMap<String, String>, SnapshotError>;
}

/// Bincode-based snapshot codec (default, production).
///
/// Uses bincode for compact binary serialization.
#[derive(Debug, Default, Clone)]
pub struct BincodeCodec;

impl SnapshotCodec for BincodeCodec {
    fn encode(&self, data: &BTreeMap<String, String>) -> Result<Vec<u8>, SnapshotError> {
        bincode::encode_to_vec(data, bincode::config::standard())
            .map_err(|e| SnapshotError::EncodeError(e.to_string()))
    }

    fn decode(&self, bytes: &[u8]) -> Result<BTreeMap<String, String>, SnapshotError> {
        if bytes.is_empty() {
            return Ok(BTreeMap::new());
        }

        let (map, _bytes_read): (BTreeMap<String, String>, usize) =
            bincode::decode_from_slice(bytes, bincode::config::standard())
                .map_err(|e| SnapshotError::DecodeError(e.to_string()))?;

        Ok(map)
    }
}

/// JSON-based snapshot codec (debugging, testing).
///
/// Uses JSON for human-readable serialization. Useful for debugging
/// snapshot contents or testing snapshot logic with readable data.
#[derive(Debug, Default, Clone)]
pub struct JsonCodec;

impl SnapshotCodec for JsonCodec {
    fn encode(&self, data: &BTreeMap<String, String>) -> Result<Vec<u8>, SnapshotError> {
        serde_json::to_vec(data).map_err(|e| SnapshotError::EncodeError(e.to_string()))
    }

    fn decode(&self, bytes: &[u8]) -> Result<BTreeMap<String, String>, SnapshotError> {
        if bytes.is_empty() {
            return Ok(BTreeMap::new());
        }

        serde_json::from_slice(bytes).map_err(|e| SnapshotError::DecodeError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());
        map.insert("special".to_string(), "with spaces and 日本語".to_string());
        map
    }

    #[test]
    fn test_bincode_roundtrip() {
        let codec = BincodeCodec;
        let original = sample_data();

        let encoded = codec.encode(&original).expect("encode failed");
        let decoded = codec.decode(&encoded).expect("decode failed");

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_bincode_empty() {
        let codec = BincodeCodec;
        let empty = BTreeMap::new();

        let encoded = codec.encode(&empty).expect("encode failed");
        let decoded = codec.decode(&encoded).expect("decode failed");

        assert_eq!(empty, decoded);
    }

    #[test]
    fn test_bincode_empty_bytes() {
        let codec = BincodeCodec;
        let decoded = codec.decode(&[]).expect("decode failed");
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_json_roundtrip() {
        let codec = JsonCodec;
        let original = sample_data();

        let encoded = codec.encode(&original).expect("encode failed");
        let decoded = codec.decode(&encoded).expect("decode failed");

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_json_human_readable() {
        let codec = JsonCodec;
        let mut data = BTreeMap::new();
        data.insert("hello".to_string(), "world".to_string());

        let encoded = codec.encode(&data).expect("encode failed");
        let json_str = String::from_utf8(encoded.clone()).expect("should be valid UTF-8");

        // JSON should be human-readable
        assert!(json_str.contains("hello"));
        assert!(json_str.contains("world"));
    }

    #[test]
    fn test_json_empty() {
        let codec = JsonCodec;
        let empty = BTreeMap::new();

        let encoded = codec.encode(&empty).expect("encode failed");
        let decoded = codec.decode(&encoded).expect("decode failed");

        assert_eq!(empty, decoded);
    }

    #[test]
    fn test_json_empty_bytes() {
        let codec = JsonCodec;
        let decoded = codec.decode(&[]).expect("decode failed");
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_bincode_decode_error() {
        let codec = BincodeCodec;
        let result = codec.decode(b"not valid bincode data!!!");
        assert!(result.is_err());
        assert!(matches!(result, Err(SnapshotError::DecodeError(_))));
    }

    #[test]
    fn test_json_decode_error() {
        let codec = JsonCodec;
        let result = codec.decode(b"not valid json {{{");
        assert!(result.is_err());
        assert!(matches!(result, Err(SnapshotError::DecodeError(_))));
    }

    #[test]
    fn test_snapshot_error_display() {
        let err = SnapshotError::EncodeError("test error".to_string());
        assert_eq!(err.to_string(), "Failed to encode snapshot: test error");

        let err = SnapshotError::DecodeError("test error".to_string());
        assert_eq!(err.to_string(), "Failed to decode snapshot: test error");
    }
}
