//! Bounded container inspection for the JSON/Zstandard replay formats.
//! This layer does not deserialize into gameplay state or submit scores.
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::Read;

pub const MAGIC: &[u8; 8] = b"PHIRAREC";
pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_EVENTS: usize = 2_000_000;
const HEADER_BYTES: usize = 12;

/// A validated, decompressed JSON payload and its container version.
#[derive(Debug)]
pub struct Container {
    pub version: u32,
    pub payload: Value,
}

/// Read only v5/v6 JSON+Zstandard containers. Legacy binary versions are
/// intentionally rejected until their dedicated cursor parsers are ported.
pub fn decode(bytes: &[u8]) -> Result<Container> {
    anyhow::ensure!(bytes.len() <= MAX_FILE_BYTES, "Replay file exceeds 128 MiB");
    anyhow::ensure!(bytes.len() >= HEADER_BYTES, "Replay header is truncated");
    anyhow::ensure!(&bytes[..8] == MAGIC, "Not a Phira replay file");

    let version = u32::from_le_bytes(bytes[8..12].try_into().context("Invalid version header")?);
    anyhow::ensure!(matches!(version, 5 | 6), "Unsupported replay version: {version}");

    let decoder = zstd::stream::read::Decoder::new(&bytes[HEADER_BYTES..]).context("Invalid Zstandard replay stream")?;
    let mut decoded = Vec::new();
    decoder
        .take((MAX_FILE_BYTES as u64) + 1)
        .read_to_end(&mut decoded)
        .context("Failed to decompress replay")?;
    anyhow::ensure!(decoded.len() <= MAX_FILE_BYTES, "Decompressed replay exceeds 128 MiB");

    let payload: Value = serde_json::from_slice(&decoded).context("Invalid replay JSON")?;
    validate_shape(&payload, version)?;
    Ok(Container { version, payload })
}

fn validate_shape(value: &Value, version: u32) -> Result<()> {
    let object = value.as_object().context("Replay JSON root must be an object")?;
    for key in ["frames", "touches", "judges"] {
        if let Some(events) = object.get(key) {
            let events = events.as_array().with_context(|| format!("Replay {key} must be an array"))?;
            anyhow::ensure!(events.len() <= MAX_EVENTS, "Replay contains too many {key} events");
        }
    }
    if version == 6 {
        anyhow::ensure!(object.get("capture").is_some_and(|v| v.is_object()), "v6 replay is missing its capture data");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(version: u32, json: &[u8]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend(zstd::stream::encode_all(json, 1).unwrap());
        bytes
    }

    #[test]
    fn reads_supported_containers_without_interpreting_score_fields() {
        let v5 = encoded(5, br#"{"frames":[],"touches":[],"judges":[],"meta":{"result":{"score":123}}}"#);
        let parsed = decode(&v5).unwrap();
        assert_eq!(parsed.version, 5);
        assert_eq!(parsed.payload["meta"]["result"]["score"], 123);

        let v6 = encoded(6, br#"{"capture":{},"frames":[],"touches":[],"judges":[]}"#);
        assert_eq!(decode(&v6).unwrap().version, 6);
    }

    #[test]
    fn rejects_invalid_header_versions_and_json() {
        assert!(decode(b"").is_err());
        assert!(decode(b"NOTAREC!0000").is_err());
        assert!(decode(&encoded(99, b"{}")).is_err());
        assert!(decode(&encoded(5, b"not-json")).is_err());
        assert!(decode(&encoded(6, br#"{"frames":[]}"#)).is_err());
    }

    #[test]
    fn rejects_wrong_shapes_and_event_overflow() {
        assert!(decode(&encoded(5, br#"{"judges":{}}"#)).is_err());
        let huge = format!(r#"{{"frames":[{}]}}"#, vec!["null"; MAX_EVENTS + 1].join(","));
        assert!(decode(&encoded(5, huge.as_bytes())).is_err());
    }

    #[test]
    fn rejects_oversized_input_before_decompression() {
        assert!(decode(&vec![0; MAX_FILE_BYTES + 1]).is_err());
    }

    #[test]
    fn caps_decompressed_size_to_prevent_bombs() {
        let bomb = encoded(5, &vec![b' '; MAX_FILE_BYTES + 1]);
        assert!(decode(&bomb).is_err());
    }
}
