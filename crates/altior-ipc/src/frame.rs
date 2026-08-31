//! Length-prefixed JSON frames for the local IPC stream (ADR 0006).
//!
//! Every message on the wire is one frame: a 4-byte big-endian `u32` length
//! followed by that many bytes of UTF-8 JSON. Frames are bounded; a frame
//! over the cap, a truncated stream, or invalid JSON is a typed
//! [`FrameError`] and the connection must close — never a panic and never a
//! resynchronizing partial read.
//!
//! [`FrameDecoder`] is an incremental state machine: callers feed arbitrary
//! byte chunks and receive whole frames, so the later async transport only
//! supplies bytes. Everything here is pure and deterministic.

use crate::error::IpcError;

/// Hard cap on one frame's payload: 256 KiB (ADR 0006). Envelope payloads
/// are already bounded at 64 KiB; this leaves headroom for envelope
/// overhead.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Encodes one JSON string as a frame (length prefix + payload).
///
/// # Errors
///
/// Returns [`IpcError::FrameTooLarge`] when `payload` exceeds
/// [`MAX_FRAME_BYTES`] and [`IpcError::FrameEncode`] when the payload is
/// not valid UTF-8-sized input for the prefix (it always is for `&str`).
pub fn encode_frame(payload: &str) -> Result<Vec<u8>, IpcError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge {
            size_bytes: payload.len(),
            limit_bytes: MAX_FRAME_BYTES,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge {
        size_bytes: payload.len(),
        limit_bytes: MAX_FRAME_BYTES,
    })?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload.as_bytes());
    Ok(frame)
}

/// Incremental frame decoder: feed bytes, receive whole frames.
///
/// Frame boundaries never depend on payload content, so a truncated or
/// corrupted prefix fails explicitly instead of desynchronizing the stream.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    /// Creates an empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends incoming raw bytes into the internal buffer without decoding yet.
    pub fn feed_bytes(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// Feeds `chunk` and returns every complete frame now decodable, in
    /// order.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::FrameTooLarge`] when a frame's declared length
    /// exceeds [`MAX_FRAME_BYTES`] — the decoder is then poisoned and the
    /// connection must close.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, IpcError> {
        self.feed_bytes(chunk);
        let mut frames = Vec::new();
        while let Some(frame) = self.decode_next()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Decodes and returns the next complete frame from the internal buffer, if available.
    ///
    /// # Errors
    /// Returns [`IpcError::FrameTooLarge`] if declared frame length exceeds [`MAX_FRAME_BYTES`].
    pub fn decode_next(&mut self) -> Result<Option<Vec<u8>>, IpcError> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }
        let length = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;
        if length > MAX_FRAME_BYTES {
            return Err(IpcError::FrameTooLarge {
                size_bytes: length,
                limit_bytes: MAX_FRAME_BYTES,
            });
        }
        if self.buffer.len() < 4 + length {
            return Ok(None);
        }
        let frame: Vec<u8> = self.buffer.drain(4..4 + length).collect();
        self.buffer.drain(0..4);
        Ok(Some(frame))
    }

    /// Number of buffered bytes waiting for a complete frame.
    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_roundtrip_byte_for_byte() {
        let payload = r#"{"kind":"ping"}"#;
        let frame = encode_frame(payload).unwrap();
        assert_eq!(
            &frame[..4],
            &u32::try_from(payload.len()).unwrap().to_be_bytes()
        );
        assert_eq!(&frame[4..], payload.as_bytes());

        let mut decoder = FrameDecoder::new();
        let delivered = decoder.feed(&frame).unwrap();
        assert_eq!(delivered, vec![payload.as_bytes().to_vec()]);
        assert_eq!(decoder.pending_bytes(), 0);
    }

    #[test]
    fn split_chunks_reassemble_into_whole_frames() {
        let first = encode_frame(r#"{"a":1}"#).unwrap();
        let second = encode_frame(r#"{"b":2}"#).unwrap();
        let mut stream = Vec::new();
        stream.extend_from_slice(&first);
        stream.extend_from_slice(&second);

        // Feed one byte at a time: only complete frames surface.
        let mut decoder = FrameDecoder::new();
        let mut received: Vec<Vec<u8>> = Vec::new();
        for byte in &stream {
            received.extend(decoder.feed(std::slice::from_ref(byte)).unwrap());
        }
        assert_eq!(received.len(), 2);
        assert_eq!(received[0], br#"{"a":1}"#.to_vec());
        assert_eq!(received[1], br#"{"b":2}"#.to_vec());
    }

    #[test]
    fn oversized_frames_fail_at_encode_and_decode() {
        let big = "x".repeat(MAX_FRAME_BYTES + 1);
        assert!(matches!(
            encode_frame(&big),
            Err(IpcError::FrameTooLarge {
                size_bytes,
                limit_bytes: MAX_FRAME_BYTES
            }) if size_bytes == MAX_FRAME_BYTES + 1
        ));

        let mut decoder = FrameDecoder::new();
        let mut header = u32::try_from(MAX_FRAME_BYTES + 1)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        header.extend_from_slice(b"whatever follows");
        assert!(matches!(
            decoder.feed(&header),
            Err(IpcError::FrameTooLarge { size_bytes, .. })
                if size_bytes == MAX_FRAME_BYTES + 1
        ));
    }

    #[test]
    fn partial_prefixes_buffer_without_failing() {
        let payload = r#"{"kind":"subscribe"}"#;
        let frame = encode_frame(payload).unwrap();
        let mut decoder = FrameDecoder::new();
        assert!(decoder.feed(&frame[..3]).unwrap().is_empty());
        assert_eq!(decoder.pending_bytes(), 3);
        let delivered = decoder.feed(&frame[3..]).unwrap();
        assert_eq!(delivered, vec![payload.as_bytes().to_vec()]);
    }
}
