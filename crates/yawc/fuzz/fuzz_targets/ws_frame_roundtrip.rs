//! Fuzz the WebSocket frame encoder/decoder round trip.
//!
//! A WebSocket client masks every frame it sends and a server unmasks it, so
//! the mask is applied twice per frame in opposite directions. Any asymmetry
//! between the two directions — a wrong rotation, a wrong tail length, an
//! off-by-one in the header size — silently corrupts payloads instead of
//! failing loudly. This target pins that invariant:
//!
//! ```text
//! Frame ──encode(client)──▶ bytes ──decode(server)──▶ Frame' with identical
//! opcode, fin flag, and payload bytes
//! ```
//!
//! The generated frame sweeps every opcode and crosses the 126-byte boundary
//! where the payload length stops fitting in the 7-bit header field. The target
//! asserts:
//!
//! * the round trip preserves `opcode`, `fin`, and payload bytes exactly;
//! * all three masking implementations agree byte for byte, and masking is an
//!   involution (`mask(mask(x)) == x`) at the generated length;
//! * control frames respect the RFC 6455 §5.5 payload ceiling;
//! * server-side decode leaves no trailing bytes for a single-frame input.

#![no_main]

use arbitrary::{Arbitrary, Result, Unstructured};
use bytes::BytesMut;
use hpx_yawc::{
    Role,
    close::CloseCode,
    codec,
    frame::{Frame, OpCode},
    mask,
};
use libfuzzer_sys::fuzz_target;
use tokio_util::codec::{Decoder as _, Encoder as _};

/// Ceiling for the decoder under test. Comfortably above the largest payload
/// this strategy generates, so a well-formed frame is never rejected for
/// exceeding the limit.
const MAX_PAYLOAD: usize = 1 << 16;

/// RFC 6455 §5.5: control frames carry at most 125 payload bytes.
const MAX_CONTROL_PAYLOAD: usize = 125;

/// Upper bound on a generated payload, keeping each iteration cheap while still
/// crossing the 126-byte boundary where the 16-bit length header kicks in.
const MAX_GENERATED_PAYLOAD: usize = 512;

fuzz_target!(|data: &[u8]| {
    let Ok(spec) = FrameSpec::arbitrary_take_rest(Unstructured::new(data)) else {
        return;
    };

    let frame = spec.to_frame();
    let opcode = frame.opcode();
    let fin = frame.is_fin();
    let payload = frame.payload().clone();

    if opcode.is_control() && payload.len() > MAX_CONTROL_PAYLOAD {
        // The encoder must reject an oversized control frame rather than emit a
        // frame a conforming peer is required to reject.
        let mut encoder = codec::Encoder::new(Role::Client);
        let mut out = BytesMut::new();
        assert!(
            encoder.encode(frame, &mut out).is_err(),
            "encoder accepted an oversized {opcode:?} control frame"
        );
        return;
    }

    let mut encoder = codec::Encoder::new(Role::Client);
    let mut wire = BytesMut::new();
    encoder
        .encode(frame, &mut wire)
        .expect("client-side encode of a well-formed frame must succeed");

    let mut decoder = codec::Decoder::new(Role::Server, MAX_PAYLOAD);
    let decoded = decoder
        .decode(&mut wire)
        .expect("server-side decode of a well-formed frame must not error")
        .expect("a fully buffered frame must decode in a single call");

    assert_eq!(decoded.opcode(), opcode, "opcode changed across the round trip");
    assert_eq!(decoded.is_fin(), fin, "fin flag changed across the round trip");
    assert_eq!(
        decoded.payload(),
        &payload,
        "payload changed across the round trip"
    );
    assert!(
        wire.is_empty(),
        "decoder left {} bytes unread for a single frame",
        wire.len()
    );

    // All masking implementations must agree, and masking must be an
    // involution at the length this input produced.
    let key = spec.mask_key;
    let mut reference = payload.to_vec();
    mask::apply_mask(&mut reference, key);

    let mut via_fast32 = payload.to_vec();
    mask::apply_mask_fast32(&mut via_fast32, key);
    assert_eq!(via_fast32, reference, "apply_mask_fast32 diverged from apply_mask");

    let mut via_fast64 = payload.to_vec();
    mask::apply_mask_fast64(&mut via_fast64, key);
    assert_eq!(via_fast64, reference, "apply_mask_fast64 diverged from apply_mask");

    mask::apply_mask(&mut reference, key);
    assert_eq!(
        reference,
        payload.to_vec(),
        "masking is not an involution at length {}",
        payload.len()
    );
});

/// Structured description of the frame to generate.
///
/// Scalar fields are drawn from the front of the input and the payload from
/// whatever remains, so even a two-byte input still produces a usable frame.
struct FrameSpec {
    fin: bool,
    opcode: OpCode,
    close_code: u16,
    payload: Vec<u8>,
    mask_key: [u8; 4],
}

impl<'a> Arbitrary<'a> for FrameSpec {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        // 0x3..=0x7 and 0xB..=0xF are reserved; `OpCode::try_from` rejects them
        // and the round trip would be meaningless, so map them onto a valid
        // opcode instead of discarding the input.
        let raw_opcode = u.int_in_range(0u8..=u8::MAX)?;
        let opcode = OpCode::try_from(raw_opcode).unwrap_or(OpCode::Binary);

        let fin = u.int_in_range(0u8..=1)? == 1;
        let close_code = u.int_in_range(u16::MIN..=u16::MAX)?;
        let mut mask_key = [0u8; 4];
        u.fill_buffer(&mut mask_key)?;

        let payload_len = u.int_in_range(0..=MAX_GENERATED_PAYLOAD)?;
        let mut payload = vec![0u8; payload_len];
        u.fill_buffer(&mut payload)?;

        Ok(Self {
            fin,
            opcode,
            close_code,
            payload,
            mask_key,
        })
    }
}

impl FrameSpec {
    /// Materialize the frame.
    ///
    /// Control frames are always emitted with `FIN` set, since RFC 6455 §5.5
    /// forbids fragmented control frames and the encoder rejects them; the
    /// generated `fin` value only applies to data frames.
    fn to_frame(&self) -> Frame {
        match self.opcode {
            OpCode::Continuation => Frame::continuation(self.payload.clone()).with_fin(self.fin),
            OpCode::Text => Frame::text(self.payload.clone()).with_fin(self.fin),
            OpCode::Binary => Frame::binary(self.payload.clone()).with_fin(self.fin),
            OpCode::Ping => Frame::ping(self.payload.clone()),
            OpCode::Pong => Frame::pong(self.payload.clone()),
            // `CloseCode::Reserved` is the escape hatch that lets an arbitrary
            // 16-bit code reach the wire through `From<CloseCode> for u16`.
            // The reason text is clipped so the resulting close payload stays
            // within the 125-byte control-frame ceiling.
            OpCode::Close => {
                let room = MAX_CONTROL_PAYLOAD.saturating_sub(2);
                let reason = &self.payload[..self.payload.len().min(room)];
                Frame::close(CloseCode::Reserved(self.close_code), reason)
            }
        }
    }
}
