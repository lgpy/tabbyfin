//! The CASTV2 wire format.
//!
//! Each frame is a 4-byte big-endian length followed by a proto2 `CastMessage` with seven fields
//! (docs/chromecast.md, "The protocol"). There is no codegen: the message is small enough that
//! hand-written encoding is shorter than a prost/protobuf build, and it keeps the crate free of a
//! C toolchain when it's cross-compiled for Android.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Error, Result};

/// The largest frame either side accepts. Devices close the connection on anything bigger, so we
/// refuse to read or write one.
pub const MAX_FRAME: usize = 64 * 1024;

/// `CastMessage.payload_type`: JSON text, which is all either side sends, or opaque bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PayloadType {
    #[default]
    Utf8,
    Binary,
}

impl PayloadType {
    fn from_wire(value: u64) -> Result<Self> {
        match value {
            0 => Ok(Self::Utf8),
            1 => Ok(Self::Binary),
            _ => Err(Error::Malformed("unknown payload type")),
        }
    }

    fn to_wire(self) -> u64 {
        match self {
            Self::Utf8 => 0,
            Self::Binary => 1,
        }
    }
}

/// A decoded `CastMessage`. `protocol_version` is 0 (CASTV2_1_0) and `payload_type` is
/// [`PayloadType::Utf8`] for every message either side sends, but both are encoded anyway: they are
/// proto2 required fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CastMessage {
    pub protocol_version: u32,
    pub source_id: String,
    pub destination_id: String,
    pub namespace: String,
    pub payload_type: PayloadType,
    pub payload_utf8: Option<String>,
    pub payload_binary: Option<Vec<u8>>,
}

impl CastMessage {
    /// A JSON message: both sides speak `payload_utf8`, with the JSON already serialized.
    pub fn json(source: &str, destination: &str, namespace: &str, payload: &str) -> Self {
        Self {
            source_id: source.to_string(),
            destination_id: destination.to_string(),
            namespace: namespace.to_string(),
            payload_utf8: Some(payload.to_string()),
            ..Self::default()
        }
    }

    /// The encoded body, without the 4-byte frame length.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(64);
        // Required fields are written even when they hold their default (0).
        put_varint(&mut out, 1, u64::from(self.protocol_version));
        put_bytes(&mut out, 2, self.source_id.as_bytes());
        put_bytes(&mut out, 3, self.destination_id.as_bytes());
        put_bytes(&mut out, 4, self.namespace.as_bytes());
        put_varint(&mut out, 5, self.payload_type.to_wire());
        if let Some(payload) = &self.payload_utf8 {
            put_bytes(&mut out, 6, payload.as_bytes());
        }
        if let Some(payload) = &self.payload_binary {
            put_bytes(&mut out, 7, payload);
        }
        if out.len() > MAX_FRAME {
            return Err(Error::FrameTooLarge {
                len: out.len(),
                max: MAX_FRAME,
            });
        }
        Ok(out)
    }

    /// Decodes a message body. Unknown fields are skipped, whatever their wire type; a truncated or
    /// otherwise malformed body is an error, so a desynchronized stream is noticed instead of
    /// misread.
    pub fn decode(body: &[u8]) -> Result<Self> {
        let mut message = Self::default();
        let mut reader = Reader { bytes: body, pos: 0 };
        while !reader.done() {
            let tag = reader.varint()?;
            let field = u32::try_from(tag >> 3).map_err(|_| Error::Malformed("field number too large"))?;
            if field == 0 {
                return Err(Error::Malformed("field number 0"));
            }
            let wire = tag & 7;
            match (field, wire) {
                (1, VARINT) => message.protocol_version = reader.varint()? as u32,
                (2, LEN) => message.source_id = reader.string()?,
                (3, LEN) => message.destination_id = reader.string()?,
                (4, LEN) => message.namespace = reader.string()?,
                (5, VARINT) => message.payload_type = PayloadType::from_wire(reader.varint()?)?,
                (6, LEN) => message.payload_utf8 = Some(reader.string()?),
                (7, LEN) => {
                    let len = reader.length()?;
                    message.payload_binary = Some(reader.take(len)?.to_vec());
                }
                // A known field with the wrong wire type is treated as an unknown one, as protobuf
                // decoders do.
                _ => reader.skip(field, wire)?,
            }
        }
        Ok(message)
    }
}

/// Reads one length-prefixed frame. The stream is left after the frame, ready for the next.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<CastMessage> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix).await?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_FRAME {
        return Err(Error::FrameTooLarge { len, max: MAX_FRAME });
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    CastMessage::decode(&body)
}

/// Writes one length-prefixed frame, in one write.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, message: &CastMessage) -> Result<()> {
    let body = message.encode()?;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

const VARINT: u64 = 0;
const FIXED64: u64 = 1;
const LEN: u64 = 2;
const START_GROUP: u64 = 3;
const END_GROUP: u64 = 4;
const FIXED32: u64 = 5;
/// Groups are deprecated (proto2 only) and a frame can't hold many, so this only stops a crafted
/// frame from recursing without end.
const MAX_GROUP_DEPTH: usize = 32;

fn put_varint(out: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(out, u64::from(field) << 3);
    write_varint(out, value);
}

fn put_bytes(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    write_varint(out, (u64::from(field) << 3) | LEN);
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        let mut shift = 0;
        while shift < 64 {
            let byte = *self.bytes.get(self.pos).ok_or(Error::Malformed("truncated varint"))?;
            self.pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
        Err(Error::Malformed("varint longer than 10 bytes"))
    }

    fn length(&mut self) -> Result<usize> {
        usize::try_from(self.varint()?).map_err(|_| Error::Malformed("length does not fit"))
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(Error::Malformed("length overflows the body"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or(Error::Malformed("truncated field"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn string(&mut self) -> Result<String> {
        let len = self.length()?;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| Error::Malformed("string field is not UTF-8"))
    }

    /// Skips one field of any wire type, so an encoder that adds a field doesn't break the decoder.
    fn skip(&mut self, field: u32, wire: u64) -> Result<()> {
        match wire {
            VARINT => {
                self.varint()?;
            }
            FIXED64 => {
                self.take(8)?;
            }
            LEN => {
                let len = self.length()?;
                self.take(len)?;
            }
            FIXED32 => {
                self.take(4)?;
            }
            START_GROUP => self.skip_group(field, 1)?,
            // 4 is only valid as the end of a group; 6 and 7 are not wire types.
            _ => return Err(Error::Malformed("unsupported wire type")),
        }
        Ok(())
    }

    fn skip_group(&mut self, start: u32, depth: usize) -> Result<()> {
        if depth > MAX_GROUP_DEPTH {
            return Err(Error::Malformed("groups nested too deeply"));
        }
        loop {
            let tag = self.varint()?;
            let field = u32::try_from(tag >> 3).map_err(|_| Error::Malformed("field number too large"))?;
            if field == 0 {
                return Err(Error::Malformed("field number 0"));
            }
            match tag & 7 {
                END_GROUP if field == start => return Ok(()),
                END_GROUP => return Err(Error::Malformed("unbalanced end of group")),
                START_GROUP => self.skip_group(field, depth + 1)?,
                wire => self.skip(field, wire)?,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect_message() -> CastMessage {
        CastMessage::json(
            "sender-0",
            "receiver-0",
            "urn:x-cast:com.google.cast.tp.connection",
            "{\"type\":\"CONNECT\"}",
        )
    }

    #[test]
    fn round_trip() {
        let message = CastMessage {
            protocol_version: 0,
            source_id: "sender-1".into(),
            destination_id: "receiver-0".into(),
            namespace: "urn:x-cast:com.google.cast.receiver".into(),
            payload_type: PayloadType::Utf8,
            payload_utf8: Some("{\"type\":\"GET_STATUS\",\"requestId\":7}".into()),
            payload_binary: None,
        };
        let encoded = message.encode().unwrap();
        assert_eq!(CastMessage::decode(&encoded).unwrap(), message);

        let binary = CastMessage {
            payload_type: PayloadType::Binary,
            payload_utf8: None,
            payload_binary: Some(vec![0, 1, 2, 255]),
            ..message
        };
        let encoded = binary.encode().unwrap();
        assert_eq!(CastMessage::decode(&encoded).unwrap(), binary);
    }

    #[test]
    fn connect_frame_bytes() {
        let body = connect_message().encode().unwrap();
        // Hand-computed: field 1 = 08 00 (tag, 0), field 2 = 12 08 + 8 bytes, field 3 = 1a 0a + 10
        // bytes, field 4 = 22 28 + 40 bytes, field 5 = 28 00, field 6 = 32 12 + 18 bytes. 2 + 10 +
        // 12 + 42 + 2 + 20 = 88 = 0x58, which is the frame length.
        let expected: Vec<u8> = [
            &[0x00, 0x00, 0x00, 0x58][..],
            b"\x08\x00",
            b"\x12\x08sender-0",
            b"\x1a\x0areceiver-0",
            b"\x22\x28urn:x-cast:com.google.cast.tp.connection",
            b"\x28\x00",
            b"\x32\x12{\"type\":\"CONNECT\"}",
        ]
        .concat();
        assert_eq!(expected.len(), 4 + body.len());

        let mut frame = Vec::new();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(write_frame(&mut frame, &connect_message()))
            .unwrap();
        assert_eq!(frame, expected);
        assert_eq!(frame.len(), 4 + 88);
    }

    #[test]
    fn zero_valued_required_fields_are_encoded() {
        let body = CastMessage::default().encode().unwrap();
        // protocol_version (field 1, varint) and payload_type (field 5, varint), both 0.
        assert_eq!(body, b"\x08\x00\x12\x00\x1a\x00\x22\x00\x28\x00");
    }

    #[test]
    fn unknown_fields_are_skipped() {
        let mut body = connect_message().encode().unwrap();
        // An unknown varint (field 9), fixed64 (field 10), length-delimited (field 11) and fixed32
        // (field 12), all with the default payload type/value.
        body.extend_from_slice(&[
            0x48, 0x01, // 9: varint 1
            0x51, 1, 2, 3, 4, 5, 6, 7, 8, // 10: fixed64
            0x5a, 0x03, b'a', b'b', b'c', // 11: bytes
            0x65, 1, 2, 3, 4, // 12: fixed32
            0x4b, 0x48, 0x01, 0x4c, // 9: a group holding one field
        ]);
        let decoded = CastMessage::decode(&body).unwrap();
        assert_eq!(decoded, connect_message());
    }

    #[tokio::test]
    async fn truncation_is_an_error() {
        // A field cut short: the tag is there, the value isn't. (A cut at a field boundary decodes
        // to a message with fewer fields, which proto2 allows, so only these are errors.)
        for cut in [
            &b"\x12"[..],              // tag, no length
            &b"\x12\x08sender"[..],    // length says 8 bytes, 6 follow
            &b"\x22"[..],              // namespace tag, nothing after it
            &b"\x32\x12{\"type\""[..], // payload cut short
        ] {
            assert!(CastMessage::decode(cut).is_err(), "{cut:?} should fail");
        }

        // A frame whose body ends early is an io error, not a partial message.
        let body = connect_message().encode().unwrap();
        let mut frame = Vec::new();
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body[..body.len() - 5]);
        assert!(matches!(read_frame(&mut frame.as_slice()).await, Err(Error::Io(_))));

        // Nor is a frame prefix that stops after two bytes.
        assert!(read_frame(&mut b"\x00\x00".as_slice()).await.is_err());
    }

    #[test]
    fn malformed_input_is_an_error() {
        // A varint that never ends.
        let mut endless = vec![0x08];
        endless.extend_from_slice(&[0x80; 11]);
        assert!(CastMessage::decode(&endless).is_err());
        // A string field that is not UTF-8.
        assert!(CastMessage::decode(b"\x12\x01\xff").is_err());
        // A field with the deprecated group wire type.
        assert!(CastMessage::decode(b"\x0b").is_err());
        // A field number of 0.
        assert!(CastMessage::decode(b"\x00\x01").is_err());
        // A payload type that doesn't exist.
        assert!(CastMessage::decode(b"\x28\x07").is_err());
    }

    #[tokio::test]
    async fn oversized_frames_are_rejected() {
        // On the way in: the length prefix alone is over the limit, so the body is never read.
        let mut prefix = Vec::new();
        prefix.extend_from_slice(&((MAX_FRAME as u32) + 1).to_be_bytes());
        assert!(matches!(
            read_frame(&mut prefix.as_slice()).await,
            Err(Error::FrameTooLarge { .. })
        ));

        // On the way out: a payload big enough to push the body over the limit.
        let big = CastMessage::json(
            "sender-0",
            "receiver-0",
            "urn:x-cast:com.google.cast.receiver",
            &"x".repeat(MAX_FRAME),
        );
        assert!(matches!(big.encode(), Err(Error::FrameTooLarge { .. })));
        let mut out = Vec::new();
        assert!(matches!(
            write_frame(&mut out, &big).await,
            Err(Error::FrameTooLarge { .. })
        ));
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn frames_read_one_at_a_time() {
        let mut stream = Vec::new();
        write_frame(&mut stream, &connect_message()).await.unwrap();
        write_frame(
            &mut stream,
            &CastMessage::json(
                "sender-0",
                "receiver-0",
                "urn:x-cast:com.google.cast.receiver",
                "{\"type\":\"GET_STATUS\"}",
            ),
        )
        .await
        .unwrap();

        let mut stream = stream.as_slice();
        assert_eq!(read_frame(&mut stream).await.unwrap(), connect_message());
        assert_eq!(
            read_frame(&mut stream).await.unwrap().payload_utf8.as_deref(),
            Some("{\"type\":\"GET_STATUS\"}")
        );
        assert!(read_frame(&mut stream).await.is_err());
    }
}
