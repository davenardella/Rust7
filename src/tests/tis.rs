// Codec tests for the experimental TIS TIMEMEAS path (crate::tis). These exercise the
// request/response byte-level encoding purely offline — no network, no PLC. See
// docs/protocol/tis-timemeas.md for what these bytes actually mean (and what is guessed).
//
// VERIFY-ON-HARDWARE: these tests confirm the codec matches what src/tis.rs and
// docs/protocol/tis-timemeas.md *claim* it does — they cannot confirm the claims themselves
// are correct against a real S7-300/400 CPU. See tests/integration/tis.rs for the
// hardware-only assumptions this can't cover offline.

use crate::tis::{
    build_timemeas_request, decode_cycle_time_guess, parse_timemeas_response, TIS_PDU_REF,
    TIS_SUBFUNC_TIMEMEAS, TIS_TYPE_GROUP_REQ,
};
use crate::S7Error;

// ── Request encoding ────────────────────────────────────────────────────────────

#[test]
fn request_is_37_bytes() {
    let req = build_timemeas_request();
    assert_eq!(req.len(), 37);
}

#[test]
fn request_tpkt_iso_header() {
    let req = build_timemeas_request();
    assert_eq!(req[0], 0x03, "TPKT version");
    let total = ((req[2] as u16) << 8) | req[3] as u16;
    assert_eq!(total, 37, "TPKT length field must equal telegram size");
    assert_eq!(req[4], 0x02, "COTP length");
    assert_eq!(req[5], 0xF0, "COTP PDU type");
    assert_eq!(req[6], 0x80, "EOT");
}

#[test]
fn request_s7_userdata_header() {
    let req = build_timemeas_request();
    assert_eq!(req[7], 0x32, "S7 protocol ID");
    assert_eq!(req[8], 0x07, "ROSCTR = Userdata");
    assert_eq!(&req[11..13], &[0x00, 0x13], "PDU reference = TIS_PDU_REF");
    assert_eq!(TIS_PDU_REF, 0x0013);
    let plen = ((req[13] as u16) << 8) | req[14] as u16;
    assert_eq!(plen, 8, "parameter length (SHORT method)");
    let dlen = ((req[15] as u16) << 8) | req[16] as u16;
    assert_eq!(dlen, 12, "data length (4-byte generic preamble + 8-byte TIS wrapper)");
}

#[test]
fn request_param_block_fields() {
    let req = build_timemeas_request();
    assert_eq!(&req[17..20], &[0x00, 0x01, 0x12], "function/itemcount/itemtype");
    assert_eq!(req[20], 0x04, "item length (SHORT sub-length)");
    assert_eq!(req[21], 0x11, "method = SHORT (single-shot)");
    assert_eq!(req[22], 0x41, "type|group = REQ(1)|TIS(1)");
    assert_eq!(req[22], TIS_TYPE_GROUP_REQ);
    assert_eq!(req[23], 0x06, "subfunction = TIMEMEAS");
    assert_eq!(req[23], TIS_SUBFUNC_TIMEMEAS);
    assert_eq!(req[24], 0x00, "sequence number = 0 (single-shot)");
}

#[test]
fn request_generic_data_preamble() {
    // Every userdata data block — request or response, any funcgroup — carries this
    // return-code/transport-size/length preamble before the funcgroup-specific content.
    // Confirmed against a real tshark dissection (see docs/protocol/tis-timemeas.md).
    let req = build_timemeas_request();
    assert_eq!(req[25], 0xFF, "return code (request-side filler, mirrors SZL's builder)");
    assert_eq!(req[26], 0x09, "transport size = OCTET_STRING");
    assert_eq!(&req[27..29], &[0x00, 0x08], "length of the TIS-specific portion that follows");
}

#[test]
fn request_tis_data_wrapper() {
    let req = build_timemeas_request();
    // TIS-specific portion starts at 29 (after the 4-byte generic preamble at 25-28):
    // parametersize(2) + datasize(2) + 4-byte zeroed parameter block.
    assert_eq!(&req[29..31], &[0x00, 0x04], "parametersize = 4");
    assert_eq!(&req[31..33], &[0x00, 0x00], "datasize = 0");
    assert_eq!(&req[33..37], &[0x00, 0x00, 0x00, 0x00], "zeroed TIS parameter registers");
}

// ── Response payload decoding ───────────────────────────────────────────────────

// Builds a hand-crafted TIS response payload (as `read_userdata_response` would hand to
// `parse_timemeas_response`): parametersize(2) + datasize(2) + res_param1(2) + res_param2(2)
// + data(datasize bytes).
fn build_response_payload(res_param1: u16, res_param2: u16, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&4u16.to_be_bytes()); // parametersize = 4
    payload.extend_from_slice(&(data.len() as u16).to_be_bytes()); // datasize
    payload.extend_from_slice(&res_param1.to_be_bytes());
    payload.extend_from_slice(&res_param2.to_be_bytes());
    payload.extend_from_slice(data);
    payload
}

#[test]
fn parse_response_minimal_no_data() {
    let payload = build_response_payload(0xAABB, 0xCCDD, &[]);
    let raw = parse_timemeas_response(&payload).expect("valid minimal response");
    assert_eq!(raw.res_param1, 0xAABB);
    assert_eq!(raw.res_param2, 0xCCDD);
    assert!(raw.data.is_empty());
}

#[test]
fn parse_response_with_data() {
    let data = vec![0u8; 20];
    let payload = build_response_payload(0x0001, 0x0002, &data);
    let raw = parse_timemeas_response(&payload).expect("valid response with data");
    assert_eq!(raw.res_param1, 0x0001);
    assert_eq!(raw.res_param2, 0x0002);
    assert_eq!(raw.data, data);
}

#[test]
fn parse_response_too_short_for_wrapper() {
    for len in 0..4 {
        let payload = vec![0u8; len];
        assert!(
            matches!(parse_timemeas_response(&payload), Err(S7Error::IsoInvalidTelegram)),
            "payload of length {len} must be rejected before any indexing"
        );
    }
}

#[test]
fn parse_response_param_size_below_minimum() {
    // parametersize = 3 (< 4): rejected before computing param_end or reading registers.
    let payload = vec![0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert!(matches!(
        parse_timemeas_response(&payload),
        Err(S7Error::IsoInvalidTelegram)
    ));
}

#[test]
fn parse_response_param_size_exceeds_payload() {
    // parametersize claims 200 bytes but the payload is far shorter.
    let mut payload = vec![0x00, 0xC8, 0x00, 0x00]; // parametersize=200, datasize=0
    payload.extend_from_slice(&[0u8; 4]); // only 4 more bytes available, not 200
    assert!(matches!(
        parse_timemeas_response(&payload),
        Err(S7Error::IsoInvalidTelegram)
    ));
}

#[test]
fn parse_response_data_size_exceeds_payload() {
    // parametersize=4 (valid), datasize claims 100 bytes but none follow.
    let payload = vec![0x00, 0x04, 0x00, 0x64, 0x00, 0x00, 0x00, 0x00];
    assert!(matches!(
        parse_timemeas_response(&payload),
        Err(S7Error::IsoInvalidTelegram)
    ));
}

// ── CycleTimeInfo guessed decode ────────────────────────────────────────────────

#[test]
fn decode_cycle_time_guess_known_values() {
    // Mirrors src/tests/szl.rs's cycle_time_parse_known_values, same guessed layout.
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(&42u32.to_be_bytes()); // ob1_count
    data[4..8].copy_from_slice(&500u32.to_be_bytes()); // min (0.1ms units)
    data[8..12].copy_from_slice(&1200u32.to_be_bytes()); // max
    data[12..16].copy_from_slice(&600u32.to_be_bytes()); // current
    let ct = decode_cycle_time_guess(&data).expect("valid 16-byte data");
    assert_eq!(ct.ob1_count, 42);
    assert!((ct.min_ms - 50.0).abs() < f64::EPSILON);
    assert!((ct.max_ms - 120.0).abs() < f64::EPSILON);
    assert!((ct.current_ms - 60.0).abs() < f64::EPSILON);
}

#[test]
fn decode_cycle_time_guess_extra_bytes_ignored() {
    let mut data = vec![0u8; 20]; // 4 extra trailing bytes
    data[0..4].copy_from_slice(&1u32.to_be_bytes());
    let ct = decode_cycle_time_guess(&data).expect("longer-than-minimum data is accepted");
    assert_eq!(ct.ob1_count, 1);
}

#[test]
fn decode_cycle_time_guess_too_short_returns_error() {
    assert!(matches!(
        decode_cycle_time_guess(&[0u8; 15]),
        Err(S7Error::IsoInvalidTelegram)
    ));
}

// ── Full guessed decode chain (payload -> TimeMeasRaw -> CycleTimeInfo) ────────

#[test]
fn full_decode_chain_plausible_response() {
    let mut cycle_bytes = vec![0u8; 16];
    cycle_bytes[0..4].copy_from_slice(&100u32.to_be_bytes());
    cycle_bytes[4..8].copy_from_slice(&10u32.to_be_bytes());
    cycle_bytes[8..12].copy_from_slice(&50u32.to_be_bytes());
    cycle_bytes[12..16].copy_from_slice(&20u32.to_be_bytes());

    let payload = build_response_payload(0x0000, 0x0000, &cycle_bytes);
    let raw = parse_timemeas_response(&payload).expect("valid payload");
    let ct = decode_cycle_time_guess(&raw.data).expect("valid cycle-time bytes");

    assert_eq!(ct.ob1_count, 100);
    assert!((ct.min_ms - 1.0).abs() < f64::EPSILON);
    assert!((ct.max_ms - 5.0).abs() < f64::EPSILON);
    assert!((ct.current_ms - 2.0).abs() < f64::EPSILON);
}
