// Experimental TIS ("Test and Installation") TIMEMEAS support for S7-300/400 cycle-time
// reads.
//
// S7-300/400 PLCs expose no SZL containing OB1 scan-cycle statistics. STEP 7's "Module
// Information -> Scan Cycle Time" retrieves this data via the TIS userdata subfunction
// TIMEMEAS (0x06, function group 1 "Programmer commands") instead. See
// docs/protocol/tis-timemeas.md for the full protocol write-up, citations, and the
// recommended DB-publish production alternative.
//
// THIS MODULE IS EXPERIMENTAL AND UNVERIFIED AGAINST REAL HARDWARE. The Wireshark s7comm
// dissector — the most complete open reference for this protocol — registers TIMEMEAS with
// the literal comment "Time measurement from to" /* never seen yet */ and does not decode
// its data part. The request parameter values and the response field layout used here are
// best-effort reconstructions of the only well-formed shape available, marked
// VERIFY-ON-HARDWARE throughout. Treat any CycleTimeInfo obtained through this path as
// provisional until confirmed against a physical S7-300/400 CPU.

use crate::client::{
    build_userdata_request, read_userdata_response, RES_SUCCESS, USERDATA_METHOD_SHORT,
};
use crate::{CycleTimeInfo, S7Error};
use std::io::Write;
use std::net::TcpStream;

/// TIS function group ("Programmer commands"). `packet-s7comm.c:673` (`S7COMM_UD_FUNCGROUP_TIS`).
pub(crate) const TIS_FUNCGROUP: u8 = 0x01;
/// TIS subfunction: TIMEMEAS ("Time measurement from to"). `packet-s7comm.c:764`.
pub(crate) const TIS_SUBFUNC_TIMEMEAS: u8 = 0x06;
/// Packed (type<<6)|funcgroup byte for a TIS *request*: type=REQ(1), group=TIS(`TIS_FUNCGROUP`).
pub(crate) const TIS_TYPE_GROUP_REQ: u8 = (1 << 6) | TIS_FUNCGROUP;
/// Arbitrary sentinel PDU reference for TIMEMEAS requests — distinct from the SZL builders'
/// 0x0011/0x0012. PDU references are client-chosen and only echoed back, never validated.
pub(crate) const TIS_PDU_REF: u16 = 0x0013;

/// VERIFY-ON-HARDWARE: the TIS parameter block's meaning for TIMEMEAS is undocumented (see
/// docs/protocol/tis-timemeas.md, "Rust7's request"). `s7comm_decode_ud_tis_param`
/// (`packet-s7comm.c:4187-4266`) only establishes that a request-side parameter block of at
/// least 4 bytes carries two generic u16 registers ("TIS Parameter 1/2") with no known
/// semantics for this subfunction. This sends the minimal well-formed 4-byte form with both
/// registers zeroed.
const TIS_TIMEMEAS_PARAM: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Builds the TIS TIMEMEAS request telegram (single-shot; see docs/protocol/tis-timemeas.md
/// "Single-shot vs. job").
///
/// VERIFY-ON-HARDWARE: the exact parameter values that select "OB1 cycle time" are unknown.
pub(crate) fn build_timemeas_request() -> Vec<u8> {
    // TIS-specific portion: parametersize(2, BE) + datasize(2, BE) + parameter block + data
    // block. `s7comm_decode_ud_tis_subfunc`, packet-s7comm.c:5213-5233. This is what
    // `s7comm_decode_ud_tis_subfunc` reads directly — i.e. everything *after* the generic
    // userdata data-block preamble below.
    let mut tis_block = Vec::with_capacity(4 + TIS_TIMEMEAS_PARAM.len());
    tis_block.extend_from_slice(&(TIS_TIMEMEAS_PARAM.len() as u16).to_be_bytes()); // parametersize = 4
    tis_block.extend_from_slice(&0u16.to_be_bytes()); // datasize = 0 (no request data block)
    tis_block.extend_from_slice(&TIS_TIMEMEAS_PARAM);

    // Generic userdata data-block preamble, shared by every function group and present in
    // both requests and responses: return code, transport size, and the length of the
    // funcgroup-specific portion that follows (`s7comm_decode_ud_data`,
    // packet-s7comm.c:6532-6543 — parsed identically regardless of funcgroup). Values mirror
    // the existing, PLC-verified SZL request builder (`build_szl_first_request`): the return
    // code is a request-side filler (a "return code" is meaningless on a request, but this
    // is what the working SZL implementation sends), transport size is OCTET_STRING.
    let mut data_block = Vec::with_capacity(4 + tis_block.len());
    data_block.push(RES_SUCCESS);
    data_block.push(0x09); // transport size = OCTET_STRING
    data_block.extend_from_slice(&(tis_block.len() as u16).to_be_bytes());
    data_block.extend_from_slice(&tis_block);

    build_userdata_request(
        TIS_PDU_REF,
        USERDATA_METHOD_SHORT,
        TIS_TYPE_GROUP_REQ,
        TIS_SUBFUNC_TIMEMEAS,
        0x00,
        &data_block,
    )
}

/// Raw, low-level result of a TIS TIMEMEAS response, before any attempt to interpret it as
/// cycle-time values. **Experimental — response layout is unverified**, see
/// docs/protocol/tis-timemeas.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimeMeasRaw {
    /// TIS response parameter register 1 (`res_param1` in the Wireshark dissector). Meaning unknown.
    pub res_param1: u16,
    /// TIS response parameter register 2 (`res_param2` in the Wireshark dissector). Meaning unknown.
    pub res_param2: u16,
    /// Raw response data-block bytes. Layout is undocumented — the dissector itself has
    /// "never seen" a TIMEMEAS response and does not decode this part.
    pub data: Vec<u8>,
}

/// Parses a TIS response payload (as returned by `read_userdata_response`) into its
/// parameter registers and raw data block. The wrapper shape (`parametersize`, `datasize`,
/// two response registers) is confirmed by the dissector (`s7comm_decode_ud_tis_subfunc`,
/// `s7comm_decode_ud_tis_param`'s response branch); the *content* of `data` is not.
pub(crate) fn parse_timemeas_response(payload: &[u8]) -> Result<TimeMeasRaw, S7Error> {
    if payload.len() < 4 {
        return Err(S7Error::IsoInvalidTelegram);
    }
    let param_size = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let data_size = u16::from_be_bytes([payload[2], payload[3]]) as usize;

    // Response-side TIS parameter block is always 2 registers (res_param1/res_param2) when
    // parametersize > 0 — packet-s7comm.c:4258-4262.
    if param_size < 4 {
        return Err(S7Error::IsoInvalidTelegram);
    }
    let param_start = 4;
    let param_end = param_start + param_size;
    if payload.len() < param_end {
        return Err(S7Error::IsoInvalidTelegram);
    }
    let res_param1 = u16::from_be_bytes([payload[param_start], payload[param_start + 1]]);
    let res_param2 = u16::from_be_bytes([payload[param_start + 2], payload[param_start + 3]]);

    let data_start = param_end;
    let data_end = data_start + data_size;
    if payload.len() < data_end {
        return Err(S7Error::IsoInvalidTelegram);
    }

    Ok(TimeMeasRaw {
        res_param1,
        res_param2,
        data: payload[data_start..data_end].to_vec(),
    })
}

/// VERIFY-ON-HARDWARE: best-effort mapping of the raw TIMEMEAS data block into
/// `CycleTimeInfo`, hypothesizing the same field shape as SZL `0x0194` on S7-1200/1500 (four
/// big-endian `u32` fields — `ob1_count`, then `min`/`max`/`current` in units of 0.1 ms) —
/// see docs/protocol/tis-timemeas.md "Rust7's response parsing". This has **not** been
/// confirmed against any real S7-300/400 TIMEMEAS response; a length-only check cannot
/// confirm the layout is actually correct.
pub(crate) fn decode_cycle_time_guess(data: &[u8]) -> Result<CycleTimeInfo, S7Error> {
    if data.len() < 16 {
        return Err(S7Error::IsoInvalidTelegram);
    }
    Ok(CycleTimeInfo {
        ob1_count: u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
        min_ms: u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as f64 / 10.0,
        max_ms: u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as f64 / 10.0,
        current_ms: u32::from_be_bytes([data[12], data[13], data[14], data[15]]) as f64 / 10.0,
    })
}

/// Sends a TIS TIMEMEAS request over `stream` and returns the decoded `CycleTimeInfo`.
///
/// **Experimental.** See the module-level documentation and docs/protocol/tis-timemeas.md —
/// the request parameters and response layout are unverified against real hardware. A
/// non-success data return code, or a response shorter than the guessed minimum length,
/// surfaces as a typed `S7Error` rather than a fabricated `CycleTimeInfo`.
pub(crate) fn read_cycle_time(
    stream: &mut TcpStream,
    pdu_length: u16,
) -> Result<CycleTimeInfo, S7Error> {
    let request = build_timemeas_request();
    stream.write_all(&request)?;
    let (payload, _continuation_byte) = read_userdata_response(stream, pdu_length)?;
    let raw = parse_timemeas_response(&payload)?;
    decode_cycle_time_guess(&raw.data)
}
