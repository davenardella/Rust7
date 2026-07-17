// Independent-dissector oracle for the TIS TIMEMEAS envelope. Writes generated request and
// synthesized response frames to a minimal pcap file and asks a real, independent tool
// (tshark, the CLI companion to Wireshark) to dissect them, asserting the s7comm dissector
// recognizes valid Userdata / Programmer-commands / Time-measurement framing rather than
// flagging the packet as malformed.
//
// This proves the envelope we construct is well-formed S7comm framing, independently of our
// own encoder/decoder. It does NOT and cannot prove TIMEMEAS's payload semantics are correct
// — see docs/protocol/tis-timemeas.md. tshark's own dissector doesn't decode TIMEMEAS's data
// part either (it has "never seen" a real frame), so this oracle checks framing validity, not
// protocol truth.
//
// Gated behind RUST7_TSHARK=1 *and* a `tshark` binary actually present on PATH — skips
// (prints a message, does not fail) otherwise, since most environments (including this
// project's default dev/CI setup) don't have Wireshark installed.

use crate::tis::{build_timemeas_request, TIS_PDU_REF, TIS_SUBFUNC_TIMEMEAS, TIS_TYPE_GROUP_REQ};
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn tshark_available() -> bool {
    if std::env::var("RUST7_TSHARK").as_deref() != Ok("1") {
        return false;
    }
    Command::new("tshark")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// Wraps a raw TPKT+COTP+S7comm byte sequence in a synthetic Ethernet+IPv4+TCP frame. pcap
// files are link-layer captures; only the TCP payload (what the s7comm dissector inspects)
// needs to be meaningful. Destination port 102 lets Wireshark's port-based dissector table
// route TCP -> TPKT -> COTP -> S7comm automatically. Checksums are left at 0 (unvalidated by
// tshark's default preferences).
fn wrap_ethernet_ipv4_tcp(tcp_payload: &[u8], src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut frame = Vec::new();

    // Ethernet header (14 bytes): dst MAC, src MAC, ethertype = IPv4.
    frame.extend_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]); // dst MAC
    frame.extend_from_slice(&[0x00, 0x66, 0x77, 0x88, 0x99, 0xAA]); // src MAC
    frame.extend_from_slice(&0x0800u16.to_be_bytes()); // ethertype: IPv4

    // IPv4 header (20 bytes, no options).
    let ip_total_len: u16 = (20 + 20 + tcp_payload.len()) as u16;
    frame.push(0x45); // version=4, IHL=5
    frame.push(0x00); // DSCP/ECN
    frame.extend_from_slice(&ip_total_len.to_be_bytes());
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // identification
    frame.extend_from_slice(&0x4000u16.to_be_bytes()); // flags=DF, fragment offset=0
    frame.push(64); // TTL
    frame.push(6); // protocol = TCP
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // header checksum (unvalidated)
    frame.extend_from_slice(&[192, 168, 0, 100]); // src IP
    frame.extend_from_slice(&[192, 168, 0, 1]); // dst IP

    // TCP header (20 bytes, no options).
    frame.extend_from_slice(&src_port.to_be_bytes());
    frame.extend_from_slice(&dst_port.to_be_bytes());
    frame.extend_from_slice(&0x00000001u32.to_be_bytes()); // sequence number
    frame.extend_from_slice(&0x00000000u32.to_be_bytes()); // ack number
    frame.push(0x50); // data offset = 5 words (20 bytes), reserved bits = 0
    frame.push(0x18); // flags: ACK, PSH
    frame.extend_from_slice(&0xFFFFu16.to_be_bytes()); // window size
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // checksum (unvalidated)
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // urgent pointer

    frame.extend_from_slice(tcp_payload);
    frame
}

// Minimal libpcap file writer: 24-byte global header + one 16-byte packet record header +
// the synthesized Ethernet frame. See https://wiki.wireshark.org/Development/LibpcapFileFormat.
fn write_pcap(path: &Path, frame: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;

    // Global header.
    file.write_all(&0xa1b2c3d4u32.to_le_bytes())?; // magic number (microsecond resolution)
    file.write_all(&2u16.to_le_bytes())?; // version major
    file.write_all(&4u16.to_le_bytes())?; // version minor
    file.write_all(&0i32.to_le_bytes())?; // thiszone (GMT)
    file.write_all(&0u32.to_le_bytes())?; // sigfigs
    file.write_all(&65535u32.to_le_bytes())?; // snaplen
    file.write_all(&1u32.to_le_bytes())?; // network = LINKTYPE_ETHERNET

    // One packet record.
    file.write_all(&0u32.to_le_bytes())?; // ts_sec
    file.write_all(&0u32.to_le_bytes())?; // ts_usec
    file.write_all(&(frame.len() as u32).to_le_bytes())?; // incl_len
    file.write_all(&(frame.len() as u32).to_le_bytes())?; // orig_len
    file.write_all(frame)?;

    Ok(())
}

// Runs `tshark -r <path> -V` and returns the full verbose dissection text.
fn dissect(path: &Path) -> String {
    let output = Command::new("tshark")
        .args(["-r", path.to_str().unwrap(), "-V"])
        .output()
        .expect("tshark invocation failed");
    assert!(
        output.status.success(),
        "tshark exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_well_formed_s7comm(dissection: &str, context: &str) {
    assert!(
        !dissection.contains("Malformed Packet"),
        "{context}: tshark flagged the frame as malformed:\n{dissection}"
    );
    assert!(
        dissection.contains("S7 Communication") || dissection.to_lowercase().contains("s7comm"),
        "{context}: tshark did not recognize the frame as S7 Communication:\n{dissection}"
    );
}

#[test]
fn tis_timemeas_request_is_well_formed_s7comm() {
    if !tshark_available() {
        eprintln!("skipping: RUST7_TSHARK=1 not set or tshark not on PATH");
        return;
    }

    let request = build_timemeas_request();
    let frame = wrap_ethernet_ipv4_tcp(&request, 50000, 102);

    let dir = std::env::temp_dir();
    let path = dir.join("rust7_tis_timemeas_request.pcap");
    write_pcap(&path, &frame).expect("failed to write pcap");

    let dissection = dissect(&path);
    assert_well_formed_s7comm(&dissection, "TIMEMEAS request");
    assert!(
        dissection.contains("Time measurement") || dissection.contains("Userdata"),
        "expected tshark to recognize the userdata funcgroup/subfunc name:\n{dissection}"
    );

    let _ = std::fs::remove_file(&path);
}

// Builds a synthetic TIS TIMEMEAS *response* frame matching the fixed-offset shape
// `read_userdata_response` actually parses (see USERDATA_SEQ_DONE_OFFSET(19) /
// USERDATA_DATA_RET_OFFSET(22) / USERDATA_DATA_LEN_OFFSET(24) / USERDATA_PAYLOAD_OFFSET(26)
// in src/client.rs, and docs/protocol/tis-timemeas.md's response envelope table).
//
// This layout was determined empirically against this test file's first (broken) attempt:
// a real tshark install caught that the initial guess — a 12-byte header (10-byte base + 2
// error-class/code bytes) followed by a 10-byte parameter block — was wrong. The confirmed
// shape is a plain 10-byte header (same as requests) followed by a 12-byte parameter block:
// the 8-byte request core (function/itemcount/itemtype/itemlen/method/type|group/subfunc/
// seqnum) plus dataUnitRef(1) + lastDataUnit(1) + errorCode(2) — all three gated together,
// matching PLC4X's `s7.mspec` optional-field condition. `USERDATA_SEQ_DONE_OFFSET` (19,
// response-relative) lands exactly on `lastDataUnit`, not `sequenceNumber` as the field name
// alone might suggest — the numeric offset was already correct (proven by the passing
// SoftPLC integration tests), only this test's understanding of *which* field it was needed
// correcting. This is a different shape than `build_userdata_request`'s SHORT(8)/EXT(12)
// request param blocks, which is why the response is hand-built here rather than reusing it.
fn build_synthetic_tis_response(pdu_ref: u16, tis_block: &[u8]) -> Vec<u8> {
    let param_len: u16 = 12;
    let data_len: u16 = (4 + tis_block.len()) as u16;
    let s7_header_len = 10;
    let total_len: u16 = (7 + s7_header_len + param_len as usize + 4 + tis_block.len()) as u16;

    let mut frame = Vec::new();
    frame.extend_from_slice(&[
        0x03,
        0x00,
        (total_len >> 8) as u8,
        (total_len & 0xFF) as u8, // TPKT
        0x02,
        0xF0,
        0x80, // COTP
        0x32, // S7 protocol ID
        0x07, // ROSCTR = Userdata
        0x00,
        0x00, // redundancy
        (pdu_ref >> 8) as u8,
        (pdu_ref & 0xFF) as u8, // PDU reference (echo)
        (param_len >> 8) as u8,
        (param_len & 0xFF) as u8, // parameter length
        (data_len >> 8) as u8,
        (data_len & 0xFF) as u8, // data length
        0x00,                    // function
        0x01,                    // item count
        0x12,                    // item type
        0x08,                    // item length
        0x12,                    // method
        (0x2 << 6) | (TIS_TYPE_GROUP_REQ & 0x3F), // type|group: RES|TIS = 0x81
        TIS_SUBFUNC_TIMEMEAS,
        0x00, // sequence number = 0
        0x00, // dataUnitRef
        0x00, // lastDataUnit = 0x00 (Yes/done) — this is USERDATA_SEQ_DONE_OFFSET
        0x00,
        0x00, // errorCode = no error
        0xFF, // data-block return code = success
        0x09, // transport size = OCTET_STRING
        (tis_block.len() as u16 >> 8) as u8,
        (tis_block.len() as u16 & 0xFF) as u8, // data-block length
    ]);
    frame.extend_from_slice(tis_block);
    frame
}

#[test]
fn tis_timemeas_response_is_well_formed_s7comm() {
    if !tshark_available() {
        eprintln!("skipping: RUST7_TSHARK=1 not set or tshark not on PATH");
        return;
    }

    // TIS wrapper (parametersize=4, datasize=16) + two response registers + 16 guessed
    // cycle-time bytes — the same shape parse_timemeas_response()/decode_cycle_time_guess()
    // expect.
    let mut tis_block = Vec::new();
    tis_block.extend_from_slice(&4u16.to_be_bytes()); // parametersize
    tis_block.extend_from_slice(&16u16.to_be_bytes()); // datasize
    tis_block.extend_from_slice(&0u16.to_be_bytes()); // res_param1
    tis_block.extend_from_slice(&0u16.to_be_bytes()); // res_param2
    tis_block.extend_from_slice(&[0u8; 16]); // guessed cycle-time bytes

    let response = build_synthetic_tis_response(TIS_PDU_REF, &tis_block);
    let frame = wrap_ethernet_ipv4_tcp(&response, 102, 50000);

    let dir = std::env::temp_dir();
    let path = dir.join("rust7_tis_timemeas_response.pcap");
    write_pcap(&path, &frame).expect("failed to write pcap");

    let dissection = dissect(&path);
    assert_well_formed_s7comm(&dissection, "TIMEMEAS response");
    assert!(
        dissection.contains("Time measurement"),
        "expected tshark to recognize the Response/TIS/TIMEMEAS parameter fields:\n{dissection}"
    );
    assert!(
        !dissection.contains("Unknown function group") && !dissection.contains("(Indication)"),
        "envelope fields misaligned — tshark fell back to an unrecognized funcgroup/type:\n{dissection}"
    );

    let _ = std::fs::remove_file(&path);
}
