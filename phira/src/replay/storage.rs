//! Bounded inspection and legacy decoding for Phira replay containers.
//! This module never deserializes replay data into gameplay state or submits scores.
use anyhow::{Context, Result};
use serde_json::{json, Value};
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

/// Typed replay data used by the library and, later, the playback scene.
/// Optional Pro-only extension objects remain opaque until their owning subsystem is ported.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct ReplayMetadata {
    pub id: String,
    pub player: Option<String>,
    pub player_id: Option<i32>,
    pub recorded_at: Option<i64>,
    pub name: String,
    pub level: String,
    pub difficulty: f32,
    pub server: Option<String>,
    pub fingerprint: Option<String>,
    pub audio_fingerprint: Option<String>,
    pub result: Option<Value>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub enum ReplayChartRef {
    Id(i32),
    Local(String),
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
pub struct ReplayTouch {
    pub phase: u8,
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub t: f32,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
pub struct ReplayJudge {
    pub t: f64,
    pub line: u32,
    pub note: u32,
    pub kind: u8,
    pub diff: f32,
    pub head_grade: Option<u8>,
}

fn default_speed() -> f32 {
    1.
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub struct ReplayTape {
    pub meta: ReplayMetadata,
    pub chart: Option<ReplayChartRef>,
    pub chart_file: Option<String>,
    pub chart_updated: Option<i64>,
    pub offset: f32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    pub mods: u32,
    pub aspect_ratio: Option<f32>,
    pub frames: Vec<f64>,
    pub touches: Vec<ReplayTouch>,
    pub judges: Vec<ReplayJudge>,
    pub has_diffs: bool,
    pub has_speed: bool,
    pub theoretical_score: bool,
    pub grading: Value,
    pub settings: Value,
    pub gameplay: Value,
    pub capture: Value,
    #[serde(skip)]
    pub source_version: u32,
    #[serde(skip)]
    pub repaired_order: bool,
}

impl Default for ReplayTape {
    fn default() -> Self {
        Self {
            meta: ReplayMetadata::default(),
            chart: None,
            chart_file: None,
            chart_updated: None,
            offset: 0.,
            speed: 1.,
            mods: 0,
            aspect_ratio: None,
            frames: Vec::new(),
            touches: Vec::new(),
            judges: Vec::new(),
            has_diffs: false,
            has_speed: false,
            theoretical_score: false,
            grading: Value::Null,
            settings: Value::Null,
            gameplay: Value::Null,
            capture: Value::Null,
            source_version: 0,
            repaired_order: false,
        }
    }
}

impl ReplayTape {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let container = decode(bytes)?;
        let mut replay: Self = serde_json::from_value(container.payload).context("Invalid replay event data")?;
        replay.source_version = container.version;
        replay.validate()?;
        replay.repaired_order = replay.frames.windows(2).any(|w| w[0] > w[1])
            || replay.judges.windows(2).any(|w| w[0].t > w[1].t)
            || replay.touches.windows(2).any(|w| w[0].t > w[1].t);
        replay.frames.sort_by(f64::total_cmp);
        replay.judges.sort_by(|a, b| a.t.total_cmp(&b.t));
        replay.touches.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(replay)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.frames.len() <= MAX_EVENTS && self.touches.len() <= MAX_EVENTS && self.judges.len() <= MAX_EVENTS,
            "Replay contains too many events"
        );
        anyhow::ensure!(self.offset.is_finite(), "Replay offset is invalid");
        anyhow::ensure!(self.speed.is_finite() && self.speed > 0. && self.speed <= 16., "Replay speed is invalid");
        anyhow::ensure!(self.aspect_ratio.is_none_or(|v| v.is_finite() && v > 0. && v <= 10.), "Replay aspect ratio is invalid");
        let valid_time = |t: f64| t.is_finite() && (-3600. ..=86400.).contains(&t);
        anyhow::ensure!(self.frames.iter().all(|t| valid_time(*t)), "Replay contains an invalid frame timestamp");
        anyhow::ensure!(
            self.touches
                .iter()
                .all(|e| valid_time(e.t as f64) && e.phase <= 2 && e.x.is_finite() && e.y.is_finite()),
            "Replay contains an invalid touch event"
        );
        anyhow::ensure!(
            self.judges.iter().all(|e| valid_time(e.t)
                && e.kind <= 9
                && e.diff.is_finite()
                && e.head_grade.is_none_or(|g| matches!(g, 0 | 1 | 2 | 3 | 4 | 7 | 8 | 9))),
            "Replay contains an invalid judgement event"
        );
        Ok(())
    }
}

/// Decode supported Pro replay versions into a bounded JSON representation.
pub fn decode(bytes: &[u8]) -> Result<Container> {
    anyhow::ensure!(bytes.len() <= MAX_FILE_BYTES, "Replay file exceeds 128 MiB");
    anyhow::ensure!(bytes.len() >= HEADER_BYTES, "Replay header is truncated");
    anyhow::ensure!(&bytes[..8] == MAGIC, "Not a Phira replay file");

    let version = u32::from_le_bytes(bytes[8..12].try_into().context("Invalid version header")?);
    let payload = match version {
        5 | 6 => {
            let decoder = zstd::stream::read::Decoder::new(&bytes[HEADER_BYTES..]).context("Invalid Zstandard replay stream")?;
            let decoded = read_limited(decoder)?;
            let payload: Value = serde_json::from_slice(&decoded).context("Invalid replay JSON")?;
            payload
        }
        3 | 4 => parse_v3_v4(bytes, version)?,
        0 | 1 => parse_jphirarec(bytes, version)?,
        _ => anyhow::bail!("Unsupported replay version: {version}"),
    };
    validate_shape(&payload, version)?;
    Ok(Container { version, payload })
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    reader
        .take((MAX_FILE_BYTES as u64) + 1)
        .read_to_end(&mut decoded)
        .context("Failed to decompress replay")?;
    anyhow::ensure!(decoded.len() <= MAX_FILE_BYTES, "Decompressed replay exceeds 128 MiB");
    Ok(decoded)
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
        anyhow::ensure!(object.get("capture").is_some_and(Value::is_object), "v6 replay is missing its capture data");
    }
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        anyhow::ensure!(count <= self.bytes.len().saturating_sub(self.pos), "Truncated replay payload");
        let start = self.pos;
        self.pos += count;
        Ok(&self.bytes[start..self.pos])
    }

    fn rest(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn f16(&mut self) -> Result<f32> {
        Ok(half_to_f32(u16::from_le_bytes(self.take(2)?.try_into()?)))
    }

    fn uleb(&mut self) -> Result<u64> {
        let mut result = 0u64;
        for shift in (0..=63).step_by(7) {
            let byte = self.u8()?;
            anyhow::ensure!(shift < 63 || byte <= 1, "ULEB128 overflow");
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
        anyhow::bail!("ULEB128 overflow")
    }

    fn str_uleb(&mut self) -> Result<String> {
        let len = usize::try_from(self.uleb()?).context("String length overflow")?;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    fn finish(self) -> Result<()> {
        anyhow::ensure!(self.rest().is_empty(), "Unexpected trailing replay data");
        Ok(())
    }
}

fn parse_v3_v4(bytes: &[u8], version: u32) -> Result<Value> {
    let mut r = Cursor::new(bytes);
    anyhow::ensure!(r.take(8)? == MAGIC, "Not a Phira replay file");
    anyhow::ensure!(r.u32()? == version, "Replay version mismatch");

    let chart = match r.u8()? {
        0 => json!({ "Id": r.i32()? }),
        1 => {
            let len = usize::from(r.u8()?);
            json!({ "Local": String::from_utf8_lossy(r.take(len)?).into_owned() })
        }
        _ => Value::Null,
    };
    let offset = r.f32()?;
    let speed = r.f32()?;
    let mods = r.u32()?;
    let grading = if version >= 4 {
        let perfect_plus = r.u8()?;
        let detailed = r.u8()?;
        anyhow::ensure!(perfect_plus <= 1 && detailed <= 1, "Invalid replay grading flags");
        json!({ "perfect_plus": perfect_plus != 0, "detailed": detailed != 0 })
    } else {
        Value::Null
    };
    let updated = r.i64()?;
    let file_len = usize::try_from(r.u32()?).context("Chart file length overflow")?;
    anyhow::ensure!(file_len <= MAX_FILE_BYTES, "Chart file reference is too large");
    let chart_file = String::from_utf8_lossy(r.take(file_len)?).into_owned();
    let touch_count = usize::try_from(r.u32()?).context("Touch count overflow")?;
    let judge_count = usize::try_from(r.u32()?).context("Judge count overflow")?;
    anyhow::ensure!(touch_count <= MAX_EVENTS && judge_count <= MAX_EVENTS, "Replay contains too many events");

    let mut touches = Vec::with_capacity(touch_count);
    for _ in 0..touch_count {
        touches.push(json!({
            "phase": r.u8()?,
            "id": r.i8()? as u8 as u64,
            "x": r.f32()?,
            "y": r.f32()?,
            "t": r.f32()?
        }));
    }
    let mut judges = Vec::with_capacity(judge_count);
    for _ in 0..judge_count {
        judges.push(json!({
            "t": r.f64()?,
            "line": r.u32()?,
            "note": r.u32()?,
            "kind": r.u8()?,
            "diff": r.f32()?,
            "head_grade": null
        }));
    }
    r.finish()?;

    let mut payload = json!({
        "chart": chart,
        "offset": offset,
        "speed": speed,
        "mods": mods,
        "touches": touches,
        "judges": judges,
        "chart_file": if chart_file.is_empty() { Value::Null } else { json!(chart_file) },
        "chart_updated": if updated == i64::MIN { Value::Null } else { json!(updated) },
        "has_speed": true
    });
    if !grading.is_null() {
        payload["grading"] = grading;
    }
    Ok(payload)
}

fn parse_jphirarec(bytes: &[u8], version: u32) -> Result<Value> {
    let mut header = Cursor::new(bytes);
    anyhow::ensure!(header.take(8)? == MAGIC, "Not a Phira replay file");
    anyhow::ensure!(header.u32()? == version, "Replay version mismatch");

    let payload = if version == 1 {
        let compression = header.u8()?;
        let compressed = header.rest();
        match compression {
            0 => compressed.to_vec(),
            1 => read_limited(zstd::stream::read::Decoder::new(compressed).context("Invalid Zstandard replay stream")?)?,
            2 => read_limited(flate2::read::ZlibDecoder::new(compressed)).context("Invalid DEFLATE replay stream")?,
            other => anyhow::bail!("Unknown replay compression type: {other}"),
        }
    } else {
        header.rest().to_vec()
    };
    anyhow::ensure!(payload.len() <= MAX_FILE_BYTES, "Replay payload exceeds 128 MiB");

    let mut r = Cursor::new(&payload);
    let record_id = r.i32()?;
    let recorded_at = if version == 1 { Some(r.i64()?) } else { None };
    let chart = r.i32()?;
    let chart_name = r.str_uleb()?;
    let user = r.i32()?;
    let user_name = r.str_uleb()?;

    let frame_count = usize::try_from(r.uleb()?).context("Frame count overflow")?;
    anyhow::ensure!(frame_count <= MAX_EVENTS, "Replay contains too many frames");
    let mut frames = Vec::with_capacity(frame_count);
    let mut touches = Vec::new();
    for _ in 0..frame_count {
        let time = r.f32()?;
        anyhow::ensure!(time.is_finite(), "Replay contains an invalid frame time");
        frames.push(f64::from(time));
        let point_count = usize::try_from(r.uleb()?).context("Touch count overflow")?;
        anyhow::ensure!(point_count <= 256 && touches.len().saturating_add(point_count) <= MAX_EVENTS, "Replay contains too many touch points");
        for _ in 0..point_count {
            let id = r.i8()?;
            let x = r.f16()?;
            let y = r.f16()? / 1.7777778;
            anyhow::ensure!(x.is_finite() && y.is_finite(), "Replay contains invalid touch coordinates");
            touches.push(json!({
                "phase": if id < 0 { 2 } else { 1 },
                "id": if id < 0 { (!id) as u64 } else { id as u64 },
                "x": x,
                "y": y,
                "t": time
            }));
        }
    }

    let judge_count = usize::try_from(r.uleb()?).context("Judge count overflow")?;
    anyhow::ensure!(judge_count <= MAX_EVENTS, "Replay contains too many judgements");
    let mut judges = Vec::with_capacity(judge_count);
    for _ in 0..judge_count {
        let time = r.f32()?;
        let line = r.u32()?;
        let note = r.u32()?;
        let legacy_kind = r.u8()?;
        let kind = match legacy_kind {
            0..=3 => legacy_kind,
            4 => 5,
            5 => 6,
            _ => anyhow::bail!("Unknown legacy judgement: {legacy_kind}"),
        };
        anyhow::ensure!(time.is_finite(), "Replay contains an invalid judgement time");
        judges.push(json!({
            "t": f64::from(time),
            "line": line,
            "note": note,
            "kind": kind,
            "diff": 0.0,
            "head_grade": null
        }));
    }
    r.finish()?;

    Ok(json!({
        "meta": {
            "id": format!("jphirarec-{user}-{record_id}"),
            "player": user_name,
            "player_id": user,
            "recorded_at": recorded_at,
            "name": chart_name
        },
        "frames": frames,
        "chart": { "Id": chart },
        "offset": 0.0,
        "speed": 1.0,
        "mods": 0,
        "touches": touches,
        "judges": judges
    }))
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15);
    let exponent = u32::from((bits >> 10) & 0x1f);
    let fraction = u32::from(bits & 0x03ff);
    if exponent == 0 {
        let value = fraction as f32 * 2f32.powi(-24);
        return if sign == 0 { value } else { -value };
    }
    if exponent == 0x1f {
        return f32::from_bits((sign << 31) | (0xff << 23) | (fraction << 13));
    }
    f32::from_bits((sign << 31) | ((exponent + 112) << 23) | (fraction << 13))
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

    fn uleb(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn jphirarec_payload() -> Vec<u8> {
        let mut p = Vec::new();
        p.extend(71i32.to_le_bytes());
        p.extend(123456789i64.to_le_bytes());
        p.extend(338i32.to_le_bytes());
        uleb(4, &mut p);
        p.extend(b"Milk");
        p.extend(999i32.to_le_bytes());
        uleb(3, &mut p);
        p.extend(b"Ken");
        uleb(1, &mut p);
        p.extend(1f32.to_le_bytes());
        uleb(2, &mut p);
        p.extend(4i8.to_le_bytes());
        p.extend(0x3c00u16.to_le_bytes());
        p.extend(0u16.to_le_bytes());
        p.extend((-5i8).to_le_bytes());
        p.extend(0u16.to_le_bytes());
        p.extend(0u16.to_le_bytes());
        uleb(1, &mut p);
        p.extend(1f32.to_le_bytes());
        p.extend(0u32.to_le_bytes());
        p.extend(0u32.to_le_bytes());
        p.push(0);
        p
    }

    #[test]
    fn reads_v5_and_v6_json_containers() {
        let v5 = encoded(5, br#"{"frames":[],"touches":[],"judges":[],"meta":{"result":{"score":123}}}"#);
        let parsed = decode(&v5).unwrap();
        assert_eq!(parsed.version, 5);
        assert_eq!(parsed.payload["meta"]["result"]["score"], 123);
        let v6 = encoded(6, br#"{"capture":{},"frames":[],"touches":[],"judges":[]}"#);
        assert_eq!(decode(&v6).unwrap().version, 6);
    }

    #[test]
    fn decodes_legacy_pro_v3_and_v4() {
        for version in [3u32, 4] {
            let mut bytes = MAGIC.to_vec();
            bytes.extend(version.to_le_bytes());
            bytes.extend([0, 42, 0, 0, 0]);
            bytes.extend(0f32.to_le_bytes());
            bytes.extend(1f32.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            if version == 4 {
                bytes.extend([1, 0]);
            }
            bytes.extend(i64::MIN.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            let parsed = decode(&bytes).unwrap();
            assert_eq!(parsed.payload["chart"]["Id"], 42);
            assert_eq!(parsed.payload["speed"], 1.0);
        }
    }

    #[test]
    fn decodes_jphirarec_v0_and_all_v1_compression_modes() {
        let payload = jphirarec_payload();
        let mut v0 = MAGIC.to_vec();
        v0.extend(0u32.to_le_bytes());
        let mut v0_payload = payload.clone();
        v0_payload.drain(4..12); // v0 predates the recorded_at field.
        v0.extend(v0_payload);
        let parsed = decode(&v0).unwrap();
        assert_eq!(parsed.payload["meta"]["player"], "Ken");
        assert_eq!(parsed.payload["judges"][0]["kind"], 0);

        for compression in 0..=2 {
            let compressed = match compression {
                0 => payload.clone(),
                1 => zstd::stream::encode_all(payload.as_slice(), 1).unwrap(),
                _ => {
                    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
                    use std::io::Write;
                    encoder.write_all(&payload).unwrap();
                    encoder.finish().unwrap()
                }
            };
            let mut bytes = MAGIC.to_vec();
            bytes.extend(1u32.to_le_bytes());
            bytes.push(compression);
            bytes.extend(compressed);
            let parsed = decode(&bytes).unwrap();
            assert_eq!(parsed.payload["meta"]["recorded_at"], 123456789i64);
            assert_eq!(parsed.payload["touches"][1]["phase"], 2);
        }
    }

    #[test]
    fn typed_decoder_validates_and_stably_orders_replay_events() {
        let json = serde_json::to_vec(&json!({
            "meta": {"name": "Typed chart", "player": "Player", "recorded_at": 1234},
            "chart": {"Id": 42},
            "frames": [2.0, 1.0],
            "touches": [
                {"phase": 0, "id": 7, "x": 0.5, "y": 0.25, "t": 2.0},
                {"phase": 1, "id": 7, "x": 0.6, "y": 0.25, "t": 1.0}
            ],
            "judges": [
                {"t": 2.0, "line": 0, "note": 1, "kind": 0, "diff": 0.01},
                {"t": 1.0, "line": 0, "note": 0, "kind": 1, "diff": -0.02}
            ]
        })).unwrap();
        let replay = ReplayTape::decode(&encoded(5, &json)).unwrap();
        assert_eq!(replay.meta.name, "Typed chart");
        assert_eq!(replay.meta.player.as_deref(), Some("Player"));
        assert_eq!(replay.source_version, 5);
        assert!(replay.repaired_order);
        assert_eq!(replay.frames, vec![1.0, 2.0]);
        assert_eq!(replay.touches[0].t, 1.0);
        assert_eq!(replay.judges[0].note, 0);
        assert!(matches!(replay.chart, Some(ReplayChartRef::Id(42))));
    }

    #[test]
    fn typed_decoder_rejects_invalid_speed_and_event_values() {
        let invalid_speed = encoded(5, br#"{"speed":0,"frames":[],"touches":[],"judges":[]}"#);
        assert!(ReplayTape::decode(&invalid_speed).is_err());

        let invalid_touch = encoded(
            5,
            br#"{"frames":[],"touches":[{"phase":3,"id":1,"x":0,"y":0,"t":0}],"judges":[]}"#,
        );
        assert!(ReplayTape::decode(&invalid_touch).is_err());

        let invalid_judge = encoded(
            5,
            br#"{"frames":[],"touches":[],"judges":[{"t":0,"line":0,"note":0,"kind":10,"diff":0}]}"#,
        );
        assert!(ReplayTape::decode(&invalid_judge).is_err());
    }

    #[test]
    fn rejects_malformed_headers_versions_shapes_and_truncation() {
        assert!(decode(b"").is_err());
        assert!(decode(b"NOTAREC!0000").is_err());
        assert!(decode(&encoded(99, b"{}")).is_err());
        assert!(decode(&encoded(5, b"not-json")).is_err());
        assert!(decode(&encoded(6, br#"{"frames":[]}"#)).is_err());
        let mut truncated = MAGIC.to_vec();
        truncated.extend(0u32.to_le_bytes());
        assert!(decode(&truncated).is_err());
    }

    #[test]
    fn rejects_wrong_shapes_event_overflow_and_oversized_input() {
        assert!(decode(&encoded(5, br#"{"judges":{}}"#)).is_err());
        let huge = format!(r#"{{"frames":[{}]}}"#, vec!["null"; MAX_EVENTS + 1].join(","));
        assert!(decode(&encoded(5, huge.as_bytes())).is_err());
        assert!(decode(&vec![0; MAX_FILE_BYTES + 1]).is_err());
    }

    #[test]
    fn bounds_decompression_and_checks_half_float_conversion() {
        let bomb = encoded(5, &vec![b' '; MAX_FILE_BYTES + 1]);
        assert!(decode(&bomb).is_err());
        assert_eq!(half_to_f32(1), 2f32.powi(-24));
        assert_eq!(half_to_f32(0x8001), -2f32.powi(-24));
    }
}
