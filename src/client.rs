// Rust7 - Native Rust S7 client (Snap7‑style) for Siemens PLCs.
// Copyright 2025 - Davide Nardella

use std::fmt;
use std::io;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use std::time::Instant;

/// Connection type: programming device (PG). Default; works for all PLC families.
pub const CT_PG: u16 = 0x0001;
/// Connection type: operator panel / HMI (OP).
pub const CT_OP: u16 = 0x0002;
/// Connection type: S7 Basic (generic S7 device).
pub const CT_S7: u16 = 0x0003;

/// Memory area: Process Inputs (PLC notation `I` / `E`). Read-only; refreshed from physical inputs each scan cycle.
pub const S7_AREA_PE: u8 = 0x81;
/// Memory area: Process Outputs (PLC notation `Q` / `A`). Written to physical outputs at end of each scan cycle.
pub const S7_AREA_PA: u8 = 0x82;
/// Memory area: Merkers / Flags (PLC notation `M`). Internal bit-addressable memory, not mapped to I/O.
pub const S7_AREA_MK: u8 = 0x83;
/// Memory area: Data Block (PLC notation `DB`). Primary target for SCADA and MES integration.
pub const S7_AREA_DB: u8 = 0x84;

/// Word length: single-bit access. Prefer [`S7Client::read_bit`] / [`S7Client::write_bit`] over passing this directly.
pub const S7_WL_BIT: u8 = 0x01;
/// Word length: byte access. Use for all byte-array reads and writes.
pub const S7_WL_BYTE: u8 = 0x02;

// Transport
const TS_RES_BIT: u8 = 0x03;
const TS_RES_BYTE: u8 = 0x04;

// PDU related
const TPKT_ISO_LEN: usize = 7; // ISO Header length
const PDU_LEN_REQ: u16 = 480; // PDU Length requested for negotiation
const ISO_CR_LEN: usize = 22; // Connection request telegram size
const ISO_CONN_REQ: u8 = 0xE0; // ISO connection requesr
const ISO_CONN_OK: u8 = 0xD0; // ISO connection accepted
const ISO_PN_REQ_LEN: usize = 25; // PDU negotiation request telegram size
const ISO_PN_RES_LEN: usize = 27; // PDU negotiation response telegram size
const ISO_ID: u8 = 0x03; // RFC 1006 ID
const S7_ID: u8 = 0x32; // S7 Protocol ID

const READ_REQ_LEN: usize = 31; // TKPT + ISO + S7 headers
const READ_RES_LEN: usize = 18; // Read job response header length
const WRITE_RES_LEN: usize = 15; // Write job response header length

const EOT: u8 = 0x80; // ISO End of Trasmission
const RW_RES_OFFSET: usize = 14;

/// Operation successful
const RES_SUCCESS: u8 = 0xFF;
/// Invalid address: read/write beyond the area limits, or the DB uses optimized access.
const RES_INVALID_ADDRESS: u8 = 0x05;
/// Resource not found
/// - The DB doesn't exists in the CPU
const RES_NOT_FOUND: u8 = 0x0A;

// SZL (System Status List) IDs
/// Module identification (order number, firmware version, PLC type).
pub const S7_SZL_CPU_ID: u16 = 0x0011;
/// Work memory information — total and used byte counts per memory area.
pub const S7_SZL_WORK_MEMORY: u16 = 0x0013;
/// Component identification (module name, serial number, AS name, copyright).
pub const S7_SZL_CPU_INFO: u16 = 0x001C;
/// Diagnostic buffer — the primary diagnostic facility accessible from outside the PLC.
pub const S7_SZL_DIAG_BUFFER: u16 = 0x00A0;
/// Cycle time statistics — OB1 min, max, and current scan cycle times.
pub const S7_SZL_CYCLE_TIME: u16 = 0x0194;
/// Current CPU operating mode (RUN / STOP / STARTUP).
pub const S7_SZL_CPU_STATUS: u16 = 0x0424;

// Userdata (ROSCTR 0x07) envelope — shared by every function group (SZL "CPU functions",
// TIS "Programmer commands", …). Only the trailing data block (request) / payload
// (response) content differs per function group. See docs/protocol/tis-timemeas.md for
// field-level citations against the Wireshark dissector and the Apache PLC4X `s7.mspec`.
const USERDATA_HEADER_LEN: usize = TPKT_ISO_LEN + 10; // TPKT+COTP (7) + S7 userdata header (10)
pub(crate) const USERDATA_METHOD_SHORT: u8 = 0x11; // first/single-shot request — no EXT continuation fields
pub(crate) const USERDATA_METHOD_EXT: u8 = 0x12; // continuation request — adds 4 EXT bytes (dataUnitRef, lastDataUnit, errorCode)

// Offsets within the S7 userdata **response** body (after the 7-byte TPKT/COTP header).
// S7 userdata response header: 12 bytes (10-byte base + 2 error-class/code bytes).
// Parameter block: 10 bytes starting at response[12].
// Data block: starts at response[22].
// Verified against moka7 / Snap7 for SZL (PDU[N] = response[N-7]); the same offsets apply
// to every function group since `s7comm_decode_ud_data` in the Wireshark dissector parses
// this return-code/transport-size/data-length preamble identically regardless of funcgroup
// — see docs/protocol/tis-timemeas.md.
const USERDATA_SEQ_DONE_OFFSET: usize = 19; // last param byte: 0x00 = final/only fragment
const USERDATA_DATA_RET_OFFSET: usize = 22; // data-block return code (0xFF = ok)
const USERDATA_DATA_LEN_OFFSET: usize = 24; // data-block payload length (2 bytes, big-endian)
const USERDATA_PAYLOAD_OFFSET: usize = 26; // start of the function-group-specific payload
const USERDATA_MAX_ITER: usize = 100; // safety cap on the fragment-accumulation loop

// Macros
macro_rules! hi_part {
    ($x:expr) => {
        (($x >> 8) & 0xFF) as u8
    };
}

macro_rules! lo_part {
    ($x:expr) => {
        ($x & 0xFF) as u8
    };
}

macro_rules! make_u16 {
    ($hi:expr, $lo:expr) => {
        ((($hi as u16) << 8) | ($lo as u16))
    };
}

/// CPU family recorded by a `connect_*` helper, used to select the correct
/// mechanism for family-specific features (e.g. cycle-time reads).
///
/// Set automatically by [`S7Client::connect_s71200_1500`] and
/// [`S7Client::connect_s7300`]. Connections made via [`S7Client::connect_rack_slot`]
/// or [`S7Client::connect_tsap`] leave it as `Other`, since the family cannot be
/// inferred from a bare rack/slot or TSAP pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuFamily {
    /// S7-1200 or S7-1500 (connected via [`S7Client::connect_s71200_1500`]).
    S71200_1500,
    /// S7-300 (connected via [`S7Client::connect_s7300`]).
    S7300,
    /// S7-400. No dedicated `connect_s7400()` helper exists (use
    /// [`S7Client::connect_rack_slot`]), so this is never set automatically —
    /// it is only produced by CPU-model-string fallback detection.
    S7400,
    /// Family not distinguishable at connect time (e.g. via [`S7Client::connect_rack_slot`]
    /// or [`S7Client::connect_tsap`]) and not resolved by fallback detection.
    Other,
}

/// Errors returned by [`S7Client`] operations.
///
/// **Low-level errors** (`Io`, `Iso*`, `TcpConnectionFailed`, `ConnectionClosed`,
/// `PduNegotiationFailed`) mean the TCP/ISO layer is broken — call [`S7Client::disconnect`]
/// before retrying. **High-level errors** (`S7NotFound`, `S7InvalidAddress`, `S7Unspecified`,
/// `SzlReadFailed`) mean the PLC rejected the request but the connection is still usable.
#[derive(Debug)]
pub enum S7Error {
    /// Underlying OS network I/O error.
    Io(io::Error),
    /// Called while not connected; use a `connect_*` method first.
    NotConnected,
    /// Could not open a TCP connection to the PLC.
    TcpConnectionFailed,
    /// PLC closed the TCP connection unexpectedly.
    ConnectionClosed,
    /// TCP connected but the ISO-on-TCP (COTP) handshake was rejected.
    IsoConnectionFailed,
    /// Received a fragmented ISO packet; the library expects complete frames only.
    IsoFragmentedPacket,
    /// ISO/TPKT header did not match the expected RFC 1006 values.
    IsoInvalidHeader,
    /// ISO telegram length or content was inconsistent.
    IsoInvalidTelegram,
    /// S7 PDU size negotiation failed, or the PLC returned an unexpected response.
    PduNegotiationFailed,
    /// A function parameter was out of range or invalid (unknown area, zero port, `bit_idx` > 7, …).
    InvalidFunParameter,
    /// Resource not found on the PLC (e.g. a Data Block that does not exist).
    S7NotFound,
    /// Invalid address: read/write beyond the Data Block limits, or the DB uses optimized access.
    S7InvalidAddress,
    /// The PLC returned an unrecognised error code.
    S7Unspecified,
    /// SZL read returned a non-success data return code from the PLC.
    SzlReadFailed,
    /// The requested feature is not available on the connected CPU family via the
    /// attempted mechanism. Not currently returned by any method — `read_cycle_time()`'s
    /// S7-300/400 branch now attempts the experimental TIS TIMEMEAS path (see
    /// `docs/protocol/tis-timemeas.md`) instead of rejecting up front. Reserved for a
    /// future capability check (e.g. SZL `0x0131`/`0x0008`) that can confirm a feature is
    /// genuinely absent rather than merely untried.
    UnsupportedCpuFamily {
        /// Human-readable name of the detected CPU family (e.g. `"S7-300/400"`).
        family: &'static str,
        /// Human-readable name of the feature that was requested.
        feature: &'static str,
    },
    /// Catch-all error with a descriptive message.
    Other(String),
}

impl fmt::Display for S7Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            S7Error::Io(e) => write!(f, "IO error: {}", e),
            S7Error::NotConnected => write!(f, "Not connected"),
            S7Error::TcpConnectionFailed => write!(f, "TCP connection failed"),
            S7Error::ConnectionClosed => write!(f, "TCP connection closed by the peer"),
            S7Error::IsoConnectionFailed => write!(f, "ISO-on-TCP connection failed"),
            S7Error::IsoFragmentedPacket => write!(f, "Fragmented ISO Packet"),
            S7Error::IsoInvalidHeader => write!(f, "Invalid ISO Header"),
            S7Error::IsoInvalidTelegram => write!(f, "Invalid ISO Telegram"),
            S7Error::PduNegotiationFailed => write!(f, "S7 PDU negotiation failed"),
            S7Error::InvalidFunParameter => write!(f, "Invalid parameter supplied to the function"),
            S7Error::S7NotFound => write!(f, "S7 Resource not found in the CPU"),
            S7Error::S7InvalidAddress => write!(f, "S7 Invalid address"),
            S7Error::S7Unspecified => write!(f, "S7 unspecified error"),
            S7Error::SzlReadFailed => write!(f, "SZL read failed (non-success data return code)"),
            S7Error::UnsupportedCpuFamily { family, feature } => {
                write!(f, "{feature} (CPU family: {family})")
            }
            S7Error::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<io::Error> for S7Error {
    fn from(err: io::Error) -> S7Error {
        S7Error::Io(err)
    }
}

/// Header fields from an SZL read response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SzlHeader {
    /// Byte length of each data record (`LENTHDR`).
    pub length_dr: u16,
    /// Total number of data records (`N_DR`), accumulated across all fragments.
    pub n_dr: u16,
}

/// Raw result of a [`S7Client::read_szl`] call.
///
/// `data` holds the concatenated record bytes from all fragments.
/// Its length is typically `header.length_dr * header.n_dr`.
#[derive(Debug, Clone)]
pub struct Szl {
    /// Parsed SZL response header.
    pub header: SzlHeader,
    /// Concatenated record payload bytes.
    pub data: Vec<u8>,
}

/// Decoded Siemens `DATE_AND_TIME` (8-byte BCD timestamp).
///
/// Corresponds to the S7 `DT` data type. All fields are decoded from BCD and validated;
/// if any BCD nibble is out of range the containing [`DiagnosticEntry::timestamp`] is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7DateTime {
    /// Full four-digit year (e.g. `2024`).
    /// BCD values 00–89 are mapped to 2000–2089; 90–99 to 1990–1999.
    pub year: u16,
    /// Month of year (1–12).
    pub month: u8,
    /// Day of month (1–31).
    pub day: u8,
    /// Hour of day (0–23).
    pub hour: u8,
    /// Minute of hour (0–59).
    pub minute: u8,
    /// Second of minute (0–59).
    pub second: u8,
    /// Milliseconds within the second (0–999).
    pub millisecond: u16,
    /// Day of week using the Siemens convention: 1 = Sunday, 2 = Monday, …, 7 = Saturday.
    pub weekday: u8,
}

/// One entry from the PLC diagnostic buffer (SZL `0x00A0`).
///
/// Each raw entry is 20 bytes on the wire: event ID (2), BCD timestamp (8), info bytes (10).
/// Entries are returned newest-first, as the PLC orders them.
///
/// Pass `event_id` to [`describe_event`](crate::describe_event) to get a human-readable
/// class and name for the event.
#[derive(Debug, Clone)]
pub struct DiagnosticEntry {
    /// Raw 16-bit event identifier. The high nibble (bits 12–15) encodes the event class;
    /// the lower 12 bits identify the specific event within that class.
    pub event_id: u16,
    /// Decoded wall-clock timestamp of the event.
    /// `None` if the 8-byte BCD field contained an invalid nibble (value > 9).
    pub timestamp: Option<S7DateTime>,
    /// The remaining 10 bytes of the 20-byte diagnostic record.
    /// Content is event-specific (priority class, related block number, etc.) and is
    /// not decoded further by this library.
    pub info: [u8; 10],
}

/// Decoded component-identification strings from SZL `0x001C` ([`S7_SZL_CPU_INFO`]).
///
/// Obtain via [`S7Client::read_cpu_info`].
#[derive(Debug, Clone, Default)]
pub struct CpuInfo {
    /// Short type designation of the module (e.g. `"CPU 1516-3 PN/DP"`).
    pub module_type_name: String,
    /// User-assigned name of the module, set in TIA Portal project properties.
    pub module_name: String,
    /// Automation station name — the PLC project name in TIA Portal.
    pub as_name: String,
    /// Copyright string embedded in the CPU firmware by Siemens.
    pub copyright: String,
    /// Unique hardware serial number of the CPU module.
    pub serial_number: String,
}

/// One memory area record from SZL `0x0013` ([`S7_SZL_WORK_MEMORY`]).
///
/// Obtain via [`S7Client::read_work_memory`].
///
/// Byte layout is derived from the Siemens S7 System and Standard Functions reference manual.
/// Validate with a Wireshark capture when switching CPU families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkMemoryRecord {
    /// Area identifier as reported by the PLC.
    pub index: u16,
    /// Area type code; encoding is CPU-family specific.
    pub area_type: u16,
    /// Total size of this memory area in bytes.
    pub total_bytes: u32,
    /// Currently used bytes in this memory area.
    pub used_bytes: u32,
}

/// Scan cycle time statistics from SZL `0x0194` ([`S7_SZL_CYCLE_TIME`]).
///
/// All times are in milliseconds. The PLC stores them in units of 0.1 ms internally.
///
/// Obtain via [`S7Client::read_cycle_time`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycleTimeInfo {
    /// Number of OB1 executions since the last CPU startup.
    pub ob1_count: u32,
    /// Minimum scan cycle time in milliseconds since the last CPU startup.
    pub min_ms: f64,
    /// Maximum scan cycle time in milliseconds since the last CPU startup.
    pub max_ms: f64,
    /// Most recently completed scan cycle time in milliseconds.
    pub current_ms: f64,
}

/// An S7 protocol client for communicating with Siemens PLCs over TCP port 102.
///
/// Create one with [`S7Client::new`], optionally call configuration methods
/// ([`set_timeout`](S7Client::set_timeout), [`set_connection_type`](S7Client::set_connection_type)),
/// then open a connection with one of the `connect_*` methods. After connecting,
/// use [`read_db`](S7Client::read_db) / [`write_db`](S7Client::write_db) for byte-level
/// Data Block access, and [`read_bit`](S7Client::read_bit) / [`write_bit`](S7Client::write_bit)
/// for individual bits.
///
/// # Example
///
/// ```no_run
/// use rust7::{S7Client, S7Error};
///
/// let mut client = S7Client::new();
/// client.connect_s71200_1500("192.168.0.100")?;
///
/// let mut buf = [0u8; 10];
/// client.read_db(1, 0, &mut buf)?;   // read 10 bytes from DB1, starting at byte 0
/// client.disconnect();
/// # Ok::<(), S7Error>(())
/// ```
pub struct S7Client {
    stream: Option<TcpStream>,
    port: u16,
    co_timeout_ms: u64,
    rd_timeout_ms: u64,
    wr_timeout_ms: u64,
    conn_type: u16,
    max_rd_pdu_data: u16,
    max_wr_pdu_data: u16,
    /// PDU size (in bytes) agreed with the PLC during connection setup.
    /// Typical value is 480. Larger values mean fewer round-trips for big reads/writes.
    pub pdu_length: u16,
    /// `true` while a TCP connection to the PLC is active.
    ///
    /// This is a cached flag — it does **not** probe the network. It stays `true` after a
    /// cable disconnect; the error only surfaces on the next read or write call.
    pub connected: bool,
    /// Duration of the most recent operation in milliseconds.
    /// Set to `0.0` when an error occurs before the operation completes.
    pub last_time: f64,
    /// Number of PDU round-trips used by the most recent read or write operation.
    /// Values above 1 mean the data was split across multiple S7 PDUs (auto-chunking).
    /// Useful for performance tuning; ignored in normal usage.
    pub chunks: usize,
    /// CPU family recorded by the `connect_*` helper used for the current connection.
    ///
    /// `None` until a `connect_*` method is called. Set to `Some(CpuFamily::S71200_1500)` by
    /// [`connect_s71200_1500`](S7Client::connect_s71200_1500), `Some(CpuFamily::S7300)` by
    /// [`connect_s7300`](S7Client::connect_s7300), and `Some(CpuFamily::Other)` by
    /// [`connect_rack_slot`](S7Client::connect_rack_slot) / [`connect_tsap`](S7Client::connect_tsap),
    /// since the family cannot be inferred from a bare rack/slot or TSAP pair.
    pub connect_profile: Option<CpuFamily>,
}

/// ### Checks the incoming ISO Packet coherence
///
/// Typically, a PLC never sends incorrect values, but we may find data in the buffer
/// from a fragmented transmission, so it is good practice to check.
///
fn check_iso_packet(
    pdu_length: u16,
    iso_packet: &mut [u8; TPKT_ISO_LEN],
) -> Result<usize, S7Error> {
    //
    //  TPKT + ISO Header
    //
    //  TPKT
    //      [0]    RFC 1006 ID          0x03
    //      [1]    Reserved             0x00
    //      [2]    HI Telegram length   Variable
    //      [3]    LO Telegram length   Variable
    //  ISO
    //      [4]    Length               0x02
    //      [5]    PDU Type             0xF0
    //      [6]    EOT                  0x80

    // Check Telegram validity

    if iso_packet[0] != ISO_ID || iso_packet[4] != 0x02 || iso_packet[5] != 0xF0 {
        return Err(S7Error::IsoInvalidHeader);
    }

    if iso_packet[6] != EOT {
        return Err(S7Error::IsoFragmentedPacket);
    }

    let telegram_length: usize = make_u16!(iso_packet[2], iso_packet[3]) as usize;

    if telegram_length < TPKT_ISO_LEN
        || telegram_length - TPKT_ISO_LEN > pdu_length as usize
        || telegram_length - TPKT_ISO_LEN == 0
    {
        return Err(S7Error::IsoInvalidTelegram);
    }

    // Returns the ramaining byte to read from the telegram
    Ok(telegram_length - TPKT_ISO_LEN)
}

// Reads and validates one ROSCTR-0x07 Userdata response fragment: TPKT/COTP header,
// data-block return code, and the function-group-specific payload bytes (everything after
// the shared return-code/transport-size/data-length preamble). This envelope shape is
// identical for every userdata function group — see the USERDATA_* offset constants above
// and docs/protocol/tis-timemeas.md. Callers must separately validate that the returned
// payload is long enough for their own function-group-specific header.
//
// Returns `(payload, continuation_byte)`. `continuation_byte == 0x00` means this was the
// last/only fragment; a non-zero value must be echoed back as `seq_num` in the next request.
pub(crate) fn read_userdata_response(
    stream: &mut TcpStream,
    pdu_length: u16,
) -> Result<(Vec<u8>, u8), S7Error> {
    let mut iso_packet = [0u8; TPKT_ISO_LEN];
    stream.read_exact(&mut iso_packet)?;

    let s7_comm_size = check_iso_packet(pdu_length, &mut iso_packet)?;

    if s7_comm_size < USERDATA_PAYLOAD_OFFSET {
        return Err(S7Error::IsoInvalidTelegram);
    }

    let mut response = [0u8; PDU_LEN_REQ as usize];
    let size_resp = stream.read(&mut response)?;
    if size_resp < s7_comm_size {
        return Err(S7Error::IsoInvalidTelegram);
    }

    if response[USERDATA_DATA_RET_OFFSET] != RES_SUCCESS {
        return Err(S7Error::SzlReadFailed);
    }

    let payload_len = make_u16!(
        response[USERDATA_DATA_LEN_OFFSET],
        response[USERDATA_DATA_LEN_OFFSET + 1]
    ) as usize;
    let payload_end = (USERDATA_PAYLOAD_OFFSET + payload_len).min(size_resp);
    let payload = if USERDATA_PAYLOAD_OFFSET < payload_end {
        response[USERDATA_PAYLOAD_OFFSET..payload_end].to_vec()
    } else {
        Vec::new()
    };

    Ok((payload, response[USERDATA_SEQ_DONE_OFFSET]))
}

// Builds a ROSCTR-0x07 Userdata request telegram: TPKT+COTP+S7 header, the userdata
// parameter-item preamble (function/itemcount/item-type/item-length/method/type|group/
// subfunc/seq-num, plus the 4 EXT continuation bytes when `method == USERDATA_METHOD_EXT`),
// followed by the caller-supplied data block. This envelope is identical for every
// function group (SZL "CPU functions" = 0x44, TIS "Programmer commands" = 0x41, …); only
// the data block's contents are function-group-specific. See
// docs/protocol/tis-timemeas.md for field-level citations against the Wireshark dissector
// and the Apache PLC4X `s7.mspec`.
pub(crate) fn build_userdata_request(
    pdu_ref: u16,
    method: u8,
    type_group: u8,
    subfunc: u8,
    seq_num: u8,
    data_block: &[u8],
) -> Vec<u8> {
    let ext = method == USERDATA_METHOD_EXT;
    let param_len: u16 = if ext { 12 } else { 8 };
    let data_len: u16 = data_block.len() as u16;
    let total_len: u16 = (USERDATA_HEADER_LEN + param_len as usize + data_block.len()) as u16;

    let mut req = Vec::with_capacity(total_len as usize);
    req.extend_from_slice(&[
        ISO_ID,
        0x00, // TPKT: version, reserved
        hi_part!(total_len),
        lo_part!(total_len), // TPKT: total length
        0x02,
        0xF0,
        EOT, // COTP: len, PDU type, EOT
        S7_ID,
        0x07, // S7: protocol ID, ROSCTR = Userdata
        0x00,
        0x00, // redundancy id
        hi_part!(pdu_ref),
        lo_part!(pdu_ref), // PDU reference
        hi_part!(param_len),
        lo_part!(param_len), // parameter length
        hi_part!(data_len),
        lo_part!(data_len), // data length
        // --- Parameter block ---
        0x00, // function
        0x01, // item count
        0x12, // item type = S7ParameterUserDataItemCPUFunctions
        if ext { 0x08 } else { 0x04 }, // item length (sub-length)
        method,
        type_group,
        subfunc,
        seq_num,
    ]);
    if ext {
        req.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // dataUnitRef, lastDataUnit, errorCode(2)
    }
    req.extend_from_slice(data_block);
    req
}

// Builds the 33-byte ROSCTR-0x07 Userdata "first" SZL read request telegram.
fn build_szl_first_request(szl_id: u16, szl_index: u16) -> Vec<u8> {
    let mut data_block = [
        RES_SUCCESS, // data return code
        0x09,        // transport size = OCTET_STRING
        0x00,
        0x04, // data payload length = 4
        0x00,
        0x00, // SZL-ID (filled below)
        0x00,
        0x00, // SZL-INDEX (filled below)
    ];
    data_block[4] = hi_part!(szl_id);
    data_block[5] = lo_part!(szl_id);
    data_block[6] = hi_part!(szl_index);
    data_block[7] = lo_part!(szl_index);
    build_userdata_request(0x0011, USERDATA_METHOD_SHORT, 0x44, 0x01, 0x00, &data_block)
}

// Builds the 33-byte ROSCTR-0x07 Userdata "next" SZL read request telegram.
// `seq_num` must echo the sequence number the PLC returned in the previous response.
fn build_szl_next_request(seq_num: u8) -> Vec<u8> {
    build_userdata_request(
        0x0012,
        USERDATA_METHOD_EXT,
        0x44,
        0x01,
        seq_num,
        &[0x0A, 0x00, 0x00, 0x00],
    )
}

// Decodes an 8-byte Siemens DATE_AND_TIME (DT) BCD timestamp.
// Returns None if any BCD nibble is invalid (> 9).
fn parse_bcd_timestamp(data: &[u8; 8]) -> Option<S7DateTime> {
    fn bcd(b: u8) -> Option<u8> {
        let hi = b >> 4;
        let lo = b & 0x0F;
        if hi > 9 || lo > 9 {
            None
        } else {
            Some(hi * 10 + lo)
        }
    }

    let year_raw = bcd(data[0])?;
    let year = if year_raw < 90 {
        2000u16 + year_raw as u16
    } else {
        1900u16 + year_raw as u16
    };
    let month = bcd(data[1])?;
    let day = bcd(data[2])?;
    let hour = bcd(data[3])?;
    let minute = bcd(data[4])?;
    let second = bcd(data[5])?;
    // Byte 6: BCD tens+units of ms (0-99). Byte 7 high nibble: hundreds of ms (0-9).
    let ms_lo = bcd(data[6])? as u16;
    let ms_hi = data[7] >> 4;
    if ms_hi > 9 {
        return None;
    }
    let millisecond = ms_hi as u16 * 100 + ms_lo;
    let weekday = data[7] & 0x0F;

    Some(S7DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        millisecond,
        weekday,
    })
}

impl S7Client {
    /// ### Creates a new `S7Client` instance with default settings.
    ///
    /// The client starts disconnected, use one of `connect_XXX` methods to open a connection to a PLC.
    ///
    /// ### Returns
    /// A new `S7Client` ready to connect.
    ///
    pub fn new() -> Self {
        S7Client {
            stream: None,
            port: 102,
            co_timeout_ms: 3000,
            rd_timeout_ms: 1000,
            wr_timeout_ms: 500,
            conn_type: CT_PG,
            max_rd_pdu_data: 0,
            max_wr_pdu_data: 0,
            pdu_length: 0x0000,
            connected: false,
            last_time: 0.0,
            chunks: 0,
            connect_profile: None,
        }
    }

    /// ### Check S7 Area validity
    ///
    fn check_area(&mut self, area: u8) -> Result<(), S7Error> {
        const AREAS: [u8; 4] = [S7_AREA_PE, S7_AREA_PA, S7_AREA_MK, S7_AREA_DB];
        if !AREAS.contains(&area) {
            return Err(S7Error::InvalidFunParameter);
        }
        Ok(())
    }

    /// ### Changes the S7 connection type to the PLC
    ///
    /// The three possible connection types are:
    /// - `CT_PG`: (as a programming device)
    /// - `CT_OP`: (as an HMI)
    /// - `CT_S7`: (as a generic device)
    ///
    /// In practice, there aren't many differences; the S7_PG connection should ensure
    /// better system responsiveness, but in reality, I've never noticed any noticeable differences.
    ///
    /// `CT_PG` is used by default.
    ///
    /// With very old PLCs (early S7300 series) that have limited communication resources,
    /// the connection may be rejected if we have S7Manager with many online windows open at the same time.
    /// In this case, use `S7_OP` or `S7_BASIC`.
    ///
    /// ### Parameters
    /// - `connection_type`: Connection type.
    ///
    /// ### Returns
    /// `Ok(())` on success, or an `S7Error` on failure.
    ///
    /// ### Errors
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    ///
    /// #### Notes
    /// 1. The client must not be connected (that is, call this method before connecting).
    /// 2. This method is not useful if you use `connect_tsap()` because the connection_type is already contained in the REMOTE_TSAP record.
    ///    
    pub fn set_connection_type(&mut self, connection_type: u16) -> Result<(), S7Error> {
        if !(CT_PG..=CT_S7).contains(&connection_type) {
            return Err(S7Error::InvalidFunParameter);
        }

        self.conn_type = connection_type;

        Ok(())
    }

    /// ### Sets operations timeout
    ///
    /// ### Parameters
    /// - `co_timeout_ms` : TCP Connection timeout (ms) (Default = 3000 ms)
    /// - `rd_timeout_ms` : Read Connection timeout (ms) (Default = 1000 ms)
    /// - `wr_timeout_ms` : Write Connection timeout (ms) (Default = 500 ms)
    ///
    /// ### Returns
    /// `Ok(())` on success, or an `S7Error` on failure.
    ///
    /// ### Errors
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    ///
    /// ### Notes
    /// 1. Values must be > 0, otherwise they are ignored
    /// 2. The client must not be connected (that is, call this method before connecting).
    ///
    pub fn set_timeout(
        &mut self,
        co_timeout_ms: u64,
        rd_timeout_ms: u64,
        wr_timeout_ms: u64,
    ) -> Result<(), S7Error> {
        if co_timeout_ms == 0 || rd_timeout_ms == 0 || wr_timeout_ms == 0 {
            return Err(S7Error::InvalidFunParameter);
        }

        self.co_timeout_ms = co_timeout_ms;
        self.rd_timeout_ms = rd_timeout_ms;
        self.wr_timeout_ms = wr_timeout_ms;

        Ok(())
    }

    /// ### Sets the TCP Connection Port
    ///
    /// The default S7 Port is 102, but if you need NAT the addresses you can use this method to change the default value.
    ///
    /// ### Parameters
    /// - `port`: TCP Connection port (1..65535)
    ///
    /// ### Returns
    /// `Ok(())` on success, or an `S7Error` on failure.
    ///
    /// ### Errors
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    ///
    /// ### Notes
    /// 1. Value must be > 0, otherwise it is ignored
    /// 2. The client must not be connected (that is, call this method before connecting).
    ///
    pub fn set_connection_port(&mut self, port: u16) -> Result<(), S7Error> {
        if port == 0 {
            return Err(S7Error::InvalidFunParameter);
        }

        self.port = port;

        Ok(())
    }

    /// Connects to an S7-1200 or S7-1500 PLC (rack 0, slot 0).
    ///
    /// ### Parameters
    /// - `ip`: PLC IPv4 address (e.g. `"192.168.0.100"`).
    ///
    /// ### Errors
    /// - `S7Error::TcpConnectionFailed`: cannot reach the PLC.
    /// - `S7Error::IsoConnectionFailed`: ISO handshake rejected.
    /// - `S7Error::PduNegotiationFailed`: PDU size negotiation failed.
    /// - `S7Error::Io`: network I/O error.
    ///
    pub fn connect_s71200_1500(&mut self, ip: &str) -> Result<(), S7Error> {
        self.connect_rack_slot(ip, 0, 0)?;
        self.connect_profile = Some(CpuFamily::S71200_1500);
        Ok(())
    }

    /// Connects to an S7-300 PLC (rack 0, slot 2).
    ///
    /// ### Parameters
    /// - `ip`: PLC IPv4 address (e.g. `"192.168.0.100"`).
    ///
    /// ### Errors
    /// Same as [`connect_s71200_1500`](S7Client::connect_s71200_1500).
    ///
    pub fn connect_s7300(&mut self, ip: &str) -> Result<(), S7Error> {
        self.connect_rack_slot(ip, 0, 2)?;
        self.connect_profile = Some(CpuFamily::S7300);
        Ok(())
    }

    /// Connects to a PLC using an explicit rack and slot (for S7-400, WinAC, PLCSIM, Sinamics).
    ///
    /// ### Parameters
    /// - `ip`: PLC IPv4 address (e.g. `"192.168.0.100"`).
    /// - `rack`: Rack number from the hardware configuration.
    /// - `slot`: Slot number of the CPU/CU within that rack.
    ///
    /// ### Errors
    /// Same as [`connect_s71200_1500`](S7Client::connect_s71200_1500).
    ///
    pub fn connect_rack_slot(&mut self, ip: &str, rack: u16, slot: u16) -> Result<(), S7Error> {
        let local_tsap: u16 = 0x0100;
        let remote_tsap: u16 = (self.conn_type << 8) + (rack * 0x20) + slot;
        self.connect_tsap(ip, local_tsap, remote_tsap)?;
        self.connect_profile = Some(CpuFamily::Other);
        Ok(())
    }

    /// Connects using explicit TSAP values — the lowest-level connection method.
    ///
    /// All other `connect_*` helpers call this internally. Use it directly only for
    /// LOGO! or S7-200, where the remote TSAP cannot be derived from rack/slot.
    ///
    /// ### Parameters
    /// - `ip`: PLC IPv4 address (e.g. `"192.168.0.100"`).
    /// - `local_tsap`: Client-side TSAP (typically `0x0100`).
    /// - `remote_tsap`: PLC-side TSAP (from LOGO! Soft Comfort or Micro/WIN).
    ///
    /// ### Errors
    /// - `S7Error::TcpConnectionFailed`: cannot open TCP connection.
    /// - `S7Error::IsoConnectionFailed`: ISO handshake rejected.
    /// - `S7Error::PduNegotiationFailed`: PDU negotiation failed.
    /// - `S7Error::Io`: network I/O error.
    ///
    pub fn connect_tsap(
        &mut self,
        ip: &str,
        local_tsap: u16,
        remote_tsap: u16,
    ) -> Result<(), S7Error> {
        self.connected = false;
        self.last_time = 0.0;
        self.connect_profile = None;
        let start_time = Instant::now();

        let addr = format!("{}:{}", ip, self.port);
        let co_timeout = Duration::from_millis(self.co_timeout_ms);
        let rd_timeout = Duration::from_millis(self.rd_timeout_ms);
        let wr_timeout = Duration::from_millis(self.wr_timeout_ms);

        let mut stream = TcpStream::connect_timeout(
            &addr
                .to_socket_addrs()?
                .next()
                .ok_or(S7Error::TcpConnectionFailed)?,
            co_timeout,
        )?;

        stream.set_read_timeout(Some(rd_timeout))?;
        stream.set_write_timeout(Some(wr_timeout))?;
        stream.set_nodelay(true)?;

        // ISO-on-TCP handshake
        let iso_cr: [u8; ISO_CR_LEN] = [
            // TPKT (RFC1006 Header)
            ISO_ID,               // RFC 1006 ID (3)
            0x00,                 // Reserved, always 0
            hi_part!(ISO_CR_LEN), // High part of packet lenght (entire frame, payload and TPDU included)
            lo_part!(ISO_CR_LEN), // Low part of packet lenght (entire frame, payload and TPDU included)
            // COTP (ISO 8073 Header)
            0x11,                  // PDU Size Length
            ISO_CONN_REQ,          // CR - Connection Request ID
            0x00,                  // Dst Reference HI
            0x00,                  // Dst Reference LO
            0x00,                  // Src Reference HI
            0x01,                  // Src Reference LO
            0x00,                  // Class + Options Flags
            0xC0,                  // PDU Max Length ID
            0x01,                  // PDU Max Length HI
            0x0A,                  // PDU Max Length LO
            0xC1,                  // Src TSAP Identifier
            0x02,                  // Src TSAP Length (2 bytes)
            hi_part!(local_tsap),  // Loc TSAP HI
            lo_part!(local_tsap),  // Loc TSAP LO
            0xC2,                  // Rem TSAP Identifier
            0x02,                  // Rem TSAP Length (2 bytes)
            hi_part!(remote_tsap), // Rem TSAP HI
            lo_part!(remote_tsap), // Rem TSAP LO
        ];

        stream.write_all(&iso_cr)?;

        let mut iso_resp = [0u8; ISO_CR_LEN];

        let size_resp = stream.read(&mut iso_resp)?;

        if size_resp < ISO_CR_LEN || iso_resp[5] != ISO_CONN_OK {
            return Err(S7Error::IsoConnectionFailed);
        }

        // S7 PDU Negotiation Telegram (contains also ISO Header and COTP Header)
        let s7_pn: [u8; ISO_PN_REQ_LEN] = [
            ISO_ID,
            0x00,
            0x00,
            0x19,
            0x02,
            0xf0,
            0x80,
            S7_ID,
            0x01,
            0x00,
            0x00,
            0x04,
            0x00,
            0x00,
            0x08,
            0x00,
            0x00,
            0xf0,
            0x00,
            0x00,
            0x01,
            0x00,
            0x01,
            hi_part!(PDU_LEN_REQ),
            lo_part!(PDU_LEN_REQ),
        ];
        stream.write_all(&s7_pn)?;
        let mut pn_resp = [0u8; ISO_PN_RES_LEN];

        let size_pn = stream.read(&mut pn_resp)?;

        if size_pn < ISO_PN_RES_LEN
            || pn_resp[0] != ISO_ID
            || pn_resp[7] != S7_ID
            || pn_resp[17] != 0x00
        {
            return Err(S7Error::PduNegotiationFailed);
        }

        self.pdu_length = make_u16!(pn_resp[25], pn_resp[26]);

        if self.pdu_length == 0 {
            return Err(S7Error::PduNegotiationFailed);
        }
        self.max_rd_pdu_data = self.pdu_length - 18; // 18 = S7 Response frame header
        self.max_wr_pdu_data = self.pdu_length - 28; // 28 = S7 Request frame header

        self.stream = Some(stream);
        self.connected = true;
        self.last_time = start_time.elapsed().as_secs_f64() * 1000.0;

        Ok(())
    }

    /// Closes the TCP connection. Safe to call when already disconnected (no-op).
    ///
    /// Always call this after a low-level error (`Io`, `IsoInvalidHeader`,
    /// `IsoFragmentedPacket`, `IsoInvalidTelegram`) before retrying — the ISO layer is
    /// stateful and the existing socket cannot be recovered.
    ///
    pub fn disconnect(&mut self) {
        if self.connected {
            // If we are disconnecting on a low-level error it's better to flush the socket
            let stream = self.stream.as_mut().unwrap();
            let _ = stream.shutdown(Shutdown::Both);
            self.stream = None;
            self.connected = false;
        }
    }

    /// ### Reads a block of data from a specific S7 memory area.
    ///
    /// ### Parameters
    /// - `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
    /// - `db_number`: DB number (ignored for non-DB areas).
    /// - `start`: Starting element index (byte index for bytes, bit index for bits).
    /// - `wordlen`: Word length constant (e.g., `S7_WL_BYTE`, `S7_WL_BIT`).
    /// - `buffer`: Destination buffer to store the read data.
    ///
    /// ### Values
    /// #### area
    /// - `S7_AREA_PE` (0x81): Process Inputs
    /// - `S7_AREA_PA` (0x82): Process Outputs
    /// - `S7_AREA_MK` (0x83): Merkers
    /// - `S7_AREA_DB` (0x84): Data Block
    /// #### wordlen
    /// - `S7_WL_BIT` (0x01) : Bit access
    /// - `S7_WL_BYTE` (0x02): Byte access
    /// #### Bit access notes
    /// 1. The start must be expressed in bits.
    ///    For example, if you want to access bit `DBX 45.3`, the start value would be 45 * 8 + 3 = 363.
    /// 2. Whatever buffer is passed, only the first byte will be used, which is considered true if !=0 or false if ==0
    ///
    /// ### Returns
    /// `Ok(())` Operation succeeded.
    ///
    /// ### Errors
    /// #### Low level
    /// - `S7Error::NotConnected`: An attempt was made to read while the client was not connected.
    /// - `S7Error::IsoInvalidHeader`: Invalid ISO Header
    /// - `S7Error::IsoInvalidTelegram`: Inconsistent expected telegram length.
    /// - `S7Error::IsoFragmentedPacket`: ISO Packet fragmented.
    /// - `S7Error::S7Unspecified`: Unknown S7 Error.
    /// - `S7Error::Io`: network I/O error.
    ///
    /// #### Suggestion
    /// In case of a low-level error, it is **highly recommended** to disconnect and reconnect the Client (as WinCC or other SCADA do)
    ///
    /// #### High level
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    /// - `S7Error::NotFound`: The resource was not found (e.g. Inexistent DB).
    /// - `S7Error::S7InvalidAddress`:
    /// 1. Attempt to read beyond the limits.
    /// 2. The DB is optimized.
    ///
    /// ### Notes
    /// - The number of bytes to read will be equal to the size of the buffer passed.
    /// - Large blocks are automatically split into chunks based on the negotiated PDU size.
    /// - In case of error the buffer contents will be inconsistent and should not be considered.
    ///
    pub fn read_area(
        &mut self,
        area: u8,
        db_number: u16,
        start: u16,
        wordlen: u8,
        buffer: &mut [u8],
    ) -> Result<(), S7Error> {
        self.last_time = 0.0;
        self.chunks = 0;

        // Check Area
        self.check_area(area)?;

        // Check Word Length
        if wordlen != S7_WL_BIT && wordlen != S7_WL_BYTE {
            return Err(S7Error::InvalidFunParameter);
        }

        // Check connection
        if !self.connected {
            return Err(S7Error::NotConnected);
        }

        let start_time = Instant::now();

        let datasize: u16 = if wordlen == S7_WL_BYTE {
            buffer.len().min(u16::MAX as usize) as u16
        } else {
            1 // Only 1 element allowed for bit operations
        };

        let stream = self.stream.as_mut().unwrap();

        let mut offset = 0;
        let mut long_start: u32 = start as u32;

        while offset < datasize {
            let remaining = datasize - offset;
            let chunk_size = remaining.min(self.max_rd_pdu_data);
            self.chunks += 1;

            // Read Request Header
            let mut request: [u8; READ_REQ_LEN] = [
                ISO_ID,
                0x00, // RFC 1006 ID (constant)                   0
                0x00,
                0x1f, // Telegram Length (31)                     2
                0x02,
                0xf0,
                0x80,  // COPT (constant)                          4
                S7_ID, // S7 Protocol ID                           7
                0x01,  // Job Type (Data)                          8
                0x00,
                0x00, // Redundancy identification                9
                0x05,
                0x00, // PDU Reference                            11
                0x00,
                0x0e, // Parameters Length (HI,LO) = 14           13
                0x00,
                0x00,                 // No write Payload here : 0                15
                0x04,                 // Function: 4 Read Var, 5 Write Var        17
                0x01,                 // Items count (used for multivar R/W)      18
                0x12,                 // Var spec.                                19
                0x0a,                 // constant 0x0a                            20
                0x10,                 // Syntax ID                                21
                wordlen,              // WordLen                                  22
                hi_part!(chunk_size), // HI (Read Payload Size)                   23
                lo_part!(chunk_size), // LO (Read Payload Size)                   24
                hi_part!(db_number),  // HI DB Number                             25
                lo_part!(db_number),  // LO DB Number                             26
                area,                 // Area                                     27
                0x00,
                0x00,
                0x00, // 24 bit Address (see below)               28
            ];

            let address = if wordlen == S7_WL_BIT {
                long_start
            } else {
                long_start << 3
            };

            request[28] = ((address >> 16) & 0xFF) as u8;
            request[29] = ((address >> 8) & 0xFF) as u8;
            request[30] = (address & 0xFF) as u8;

            stream.write_all(&request)?;

            // Read and check ISO header
            let mut iso_packet = [0u8; TPKT_ISO_LEN];
            stream.read_exact(&mut iso_packet)?;

            let s7_comm_size = check_iso_packet(self.pdu_length, &mut iso_packet)?;

            if s7_comm_size < READ_RES_LEN {
                return Err(S7Error::IsoInvalidTelegram);
            }

            // Read and check S7 Telegram
            let mut response = [0u8; PDU_LEN_REQ as usize];
            let size_resp = stream.read(&mut response)?;

            if size_resp < s7_comm_size {
                return Err(S7Error::IsoInvalidTelegram);
            }

            if response[RW_RES_OFFSET] != RES_SUCCESS {
                match response[RW_RES_OFFSET] {
                    RES_NOT_FOUND => return Err(S7Error::S7NotFound),
                    RES_INVALID_ADDRESS => return Err(S7Error::S7InvalidAddress),
                    _ => return Err(S7Error::S7Unspecified),
                }
            }

            // Copy payload
            let payload = &response
                [READ_RES_LEN..READ_RES_LEN + (size_resp - READ_RES_LEN).min(chunk_size as usize)];
            buffer[offset as usize..offset as usize + payload.len()].copy_from_slice(payload);

            offset += chunk_size;
            long_start += chunk_size as u32;
        }

        self.last_time = start_time.elapsed().as_secs_f64() * 1000.0;

        Ok(())
    }

    /// ### Writes a block of data to a specific S7 memory area.
    ///
    /// ### Parameters
    /// - `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
    /// - `db_number`: DB number (ignored for non-DB areas).
    /// - `start`: Starting element index (byte index for bytes, bit index for bits).
    /// - `wordlen`: Word length constant (e.g., `S7_WL_BYTE`, `S7_WL_BIT`).
    /// - `buffer`: Source buffer to write.
    ///
    /// ### Values
    /// #### area
    /// - `S7_AREA_PE` (0x81): Process Inputs
    /// - `S7_AREA_PA` (0x82): Process Outputs
    /// - `S7_AREA_MK` (0x83): Merkers
    /// - `S7_AREA_DB` (0x84): Data Block
    /// #### wordlen
    /// - `S7_WL_BIT` (0x01) : Bit access
    /// - `S7_WL_BYTE` (0x02): Byte access
    /// #### Bit access notes
    /// 1. The start must be expressed in bits.
    ///    For example, if you want to access bit `DBX 45.3`, the start value would be 45 * 8 + 3 = 363.
    /// 2. Whatever buffer is passed, only the first byte will be used, which is considered true if !=0 or false if ==0
    /// 3. Writing a bit affects **only that bit**, leaving adjacent bits in the byte unchanged.
    ///
    /// ### Returns
    /// `Ok(())` Operation succeeded.
    ///
    /// ### Errors
    /// #### Low level
    /// - `S7Error::NotConnected`: An attempt was made to write while the client was not connected.
    /// - `S7Error::IsoInvalidHeader`: Invalid ISO Header
    /// - `S7Error::IsoInvalidTelegram`: Inconsistent expected telegram length.
    /// - `S7Error::IsoFragmentedPacket`: ISO Packet fragmented.
    /// - `S7Error::S7Unspecified`: Unknown S7 Error.
    /// - `S7Error::Io`: network I/O error.
    ///
    /// #### Suggestion
    /// In case of a low-level error, it is **highly recommended** to disconnect and reconnect the Client (as WinCC or other SCADA do)
    ///
    /// #### High level
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    /// - `S7Error::NotFound`: The resource was not found (e.g. Inexistent DB).
    /// - `S7Error::S7InvalidAddress`:
    /// 1. Attempt to write beyond the limits.
    /// 2. The DB is optimized.
    ///
    /// ### Notes
    /// - The number of bytes to write will be equal to the size of the buffer passed.
    /// - Large blocks are automatically split into chunks based on the negotiated PDU size.
    /// - Writing the output buffer (`S7_AREA_PA`) usually does not produce useful results, in fact the output process image
    ///   will be rewritten by OB1 in the next round
    ///
    pub fn write_area(
        &mut self,
        area: u8,
        db_number: u16,
        start: u16,
        wordlen: u8,
        buffer: &[u8],
    ) -> Result<(), S7Error> {
        self.last_time = 0.0;
        self.chunks = 0;

        // Check Area
        self.check_area(area)?;

        // Check Word Length
        if wordlen != S7_WL_BIT && wordlen != S7_WL_BYTE {
            return Err(S7Error::InvalidFunParameter);
        }

        // Check connection
        if !self.connected {
            return Err(S7Error::NotConnected);
        }

        let start_time = Instant::now();
        let stream = self.stream.as_mut().unwrap();
        let mut offset = 0;
        let mut long_start: u32 = start as u32;

        let datasize: usize = if wordlen == S7_WL_BYTE {
            buffer.len().min(u16::MAX as usize)
        } else {
            1 // Only 1 element allowed for bit operations
        };

        let transport: u8 = if wordlen == S7_WL_BIT {
            TS_RES_BIT
        } else {
            TS_RES_BYTE
        };

        while offset < datasize {
            self.chunks += 1;
            let chunk_size = (datasize - offset).min(self.max_wr_pdu_data as usize);
            let chunk = &buffer[offset..offset + chunk_size];

            let bits_payload: u16 = if wordlen == S7_WL_BIT {
                1
            } else {
                (chunk_size << 3) as u16
            };

            // 35 byte Write Request Header
            let mut request = vec![
                ISO_ID,
                0x00, // RFC 1006 ID (constant)
                0x00,
                0x00, // Telegram Length (HI,LO) = Payload Size + 35
                0x02,
                0xf0,
                0x80,  // COPT (constant)
                S7_ID, // S7 Protocol ID
                0x01,  // Job Type (Data)
                0x00,
                0x00, // Redundancy identification
                0x05,
                0x00, // PDU Reference
                0x00,
                0x0e,                     // Parameters Length (HI,LO) = 14
                hi_part!(chunk_size + 4), // HI (Payload Size + 4)
                lo_part!(chunk_size + 4), // LO (Payload Size + 4)
                0x05,                     // Function: 4 Read Var, 5 Write Var
                0x01,                     // Items count (used for multivar R/W)
                0x12,                     // Var spec.
                0x0a,                     // constant 0x0a
                0x10,                     // Syntax ID
                wordlen,
                hi_part!(chunk_size), // HI Payload size
                lo_part!(chunk_size), // LO Payload size
                hi_part!(db_number),  // HI DB Number
                lo_part!(db_number),  // LO DB Number
                area,                 // Area ID
                0x00,
                0x00,
                0x00,                   // 24 bit Address (see below)
                0x00,                   // Reserved
                transport,              // TS_RES_BIT or TS_RES_BYTE
                hi_part!(bits_payload), // HI Payload size (bits)
                lo_part!(bits_payload), // LO Payload size (bits)
            ];

            request.extend_from_slice(chunk); // Append the Payload to the Header

            let total_len = request.len();

            // Set Telegram length
            request[2] = hi_part!(total_len);
            request[3] = lo_part!(total_len);

            // Set Start Address (bits) inside the area
            let address = if wordlen == S7_WL_BIT {
                long_start
            } else {
                long_start << 3
            };

            request[28] = ((address >> 16) & 0xFF) as u8;
            request[29] = ((address >> 8) & 0xFF) as u8;
            request[30] = (address & 0xFF) as u8;

            stream.write_all(&request)?;

            // Read and check ISO header
            let mut iso_packet = [0u8; TPKT_ISO_LEN];
            stream.read_exact(&mut iso_packet)?;

            let s7_comm_size = check_iso_packet(self.pdu_length, &mut iso_packet)?;

            if s7_comm_size < WRITE_RES_LEN {
                return Err(S7Error::IsoInvalidTelegram);
            }

            // Read and check S7 Telegram
            let mut response = [0u8; PDU_LEN_REQ as usize];
            let size_resp = stream.read(&mut response)?;

            if size_resp < s7_comm_size {
                return Err(S7Error::IsoInvalidTelegram);
            }

            if response[RW_RES_OFFSET] != RES_SUCCESS {
                match response[RW_RES_OFFSET] {
                    RES_NOT_FOUND => return Err(S7Error::S7NotFound),
                    RES_INVALID_ADDRESS => return Err(S7Error::S7InvalidAddress),
                    _ => return Err(S7Error::S7Unspecified),
                }
            }

            // Next Chunk
            offset += chunk_size;
            long_start += chunk_size as u32;
        }

        self.last_time = start_time.elapsed().as_secs_f64() * 1000.0;

        Ok(())
    }

    /// ### Reads a block of byte from a specific Data Block (DB)
    ///
    /// This helper method is same as `read_area()` with:
    /// - area = `S7_AREA_DB`
    /// - wordlen = `S7_WL_BYTE`
    ///
    /// ### Parameters
    /// - `db_number`: DB number
    /// - `start`: Starting byte index
    /// - `buffer`: Destination buffer to store the read data.
    ///
    /// ### Notes
    /// - The number of bytes to read will be equal to the size of the buffer passed.
    /// ---
    /// For further info, please refer to `read_area()`
    ///
    pub fn read_db(
        &mut self,
        db_number: u16,
        start: u16,
        buffer: &mut [u8],
    ) -> Result<(), S7Error> {
        self.read_area(S7_AREA_DB, db_number, start, S7_WL_BYTE, buffer)
    }

    /// ### Reads a bit from a specific S7 memory area
    ///
    /// This helper method is same as `read_area()` with:
    /// - wordlen = `S7_WL_BIT`
    /// - start = `byte_num * 8 + bit_idx`
    ///
    /// ### Parameters
    /// - `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
    /// - `db_number`: DB number (ignored for non-DB areas).
    /// - `byte_num`: Byte Number.
    /// - `bit_idx`: Bit index inside the byte (0..7).
    ///
    /// ### Example
    /// To read DB10.DBX71.4 use:
    ///
    /// ```my_bit = read_bit(S7_AREA_DB, 10, 71, 4);```
    ///
    /// ### Returns
    /// `Ok(<bool>)` or `Err(<S7Error>)`
    ///
    /// ### Errors
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    /// - Other reported by read_area()
    ///
    /// ### Suggestion
    ///
    /// ```text
    /// Even reading a single bit requires an entire telegram.
    /// Since reading is non-invasive, if you need to read multiple bits
    /// (more or less adjacent in the same area), I recommend reading blocks
    /// of bytes and then unpacking them.
    /// ```
    /// ---
    /// For further info, please refer to `read_area()`
    ///
    pub fn read_bit(
        &mut self,
        area: u8,
        db_number: u16,
        byte_num: u16,
        bit_idx: u8,
    ) -> Result<bool, S7Error> {
        if bit_idx > 7 {
            return Err(S7Error::InvalidFunParameter);
        }

        let start: u16 = byte_num * 8 + bit_idx as u16;
        let mut buffer = [0u8; 1];

        self.read_area(area, db_number, start, S7_WL_BIT, &mut buffer)?;

        Ok(buffer[0] != 0)
    }

    /// ### Writes a block of byte to a specific Data Block (DB)
    ///
    /// This helper method is same as `write_area()` with:
    /// - area = `S7_AREA_DB`
    /// - wordlen = `S7_WL_BYTE`
    ///
    /// ### Parameters
    /// - `db_number`: DB number
    /// - `start`: Starting byte index
    /// - `buffer`: Source buffer to write.
    ///
    /// ### Notes
    /// - The number of bytes to write will be equal to the size of the buffer passed.
    /// ---
    /// For further info, please refer to `write_area()`
    ///
    pub fn write_db(&mut self, db_number: u16, start: u16, buffer: &[u8]) -> Result<(), S7Error> {
        self.write_area(S7_AREA_DB, db_number, start, S7_WL_BYTE, buffer)
    }

    /// ### Writes a bit to a specific S7 memory area
    ///
    /// This helper method is same as `write_area()` with:
    /// - wordlen = `S7_WL_BIT`
    /// - start = `byte_num * 8 + bit_idx`
    ///
    /// ### Parameters
    /// - `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
    /// - `db_number`: DB number (ignored for non-DB areas).
    /// - `byte_num`: Byte Number.
    /// - `bit_idx`: Bit index inside the byte (0..7).
    /// - `value`: Value to write (true | false).
    ///
    /// ### Example
    /// To write **1** into DB10.DBX71.4 use:
    ///
    /// ```write_bit(S7_AREA_DB, 10, 71, 4, true);```
    ///
    /// ### Returns
    /// `Ok(())` Operation succeeded.
    ///
    /// ### Errors
    /// - `S7Error::InvalidFunParam`: Invalid parameter supplied to the function.
    /// - Other reported by read_area()
    ///
    /// ### Notes
    /// ```text
    /// Writing a bit affects only that bit, leaving adjacent bits in the byte unchanged.
    /// ```
    /// ---
    /// For further info, please refer to `write_area()`
    ///
    pub fn write_bit(
        &mut self,
        area: u8,
        db_number: u16,
        byte_num: u16,
        bit_idx: u8,
        value: bool,
    ) -> Result<(), S7Error> {
        if bit_idx > 7 {
            return Err(S7Error::InvalidFunParameter);
        }

        let start: u16 = byte_num * 8 + bit_idx as u16;
        let mut data = [0u8; 1];
        data[0] = value as u8;

        self.write_area(area, db_number, start, S7_WL_BIT, &data)
    }

    // ── SZL (System Status List) ─────────────────────────────────────────────

    // Core SZL read: sends FIRST + optional NEXT telegrams (ROSCTR 0x07 Userdata),
    // accumulates fragments, returns the raw Szl with header and concatenated records.
    fn read_szl_block(&mut self, szl_id: u16, szl_index: u16) -> Result<Szl, S7Error> {
        self.last_time = 0.0;
        self.chunks = 0;

        if !self.connected {
            return Err(S7Error::NotConnected);
        }

        let start_time = Instant::now();
        let stream = self.stream.as_mut().unwrap();

        let mut records: Vec<u8> = Vec::new();
        let mut length_dr: u16 = 0;
        let mut n_dr: u16 = 0;
        let mut is_first = true;
        let mut seq_num: u8 = 0;

        for _ in 0..USERDATA_MAX_ITER {
            self.chunks += 1;

            let request = if is_first {
                build_szl_first_request(szl_id, szl_index)
            } else {
                build_szl_next_request(seq_num)
            };

            stream.write_all(&request)?;

            let (payload, done_byte) = read_userdata_response(stream, self.pdu_length)?;

            if is_first {
                // FIRST response SZL payload layout:
                //   [0..2]  SZL-ID
                //   [2..4]  SZL-INDEX
                //   [4..6]  LENTHDR (bytes per record)
                //   [6..8]  N_DR (total record count across all fragments)
                //   [8..]   records
                if payload.len() < 8 {
                    return Err(S7Error::IsoInvalidTelegram);
                }
                length_dr = make_u16!(payload[4], payload[5]);
                n_dr = make_u16!(payload[6], payload[7]);
                records.extend_from_slice(&payload[8..]);
            } else {
                // NEXT response SZL payload layout:
                //   [0..2]  LENTHDR (repeat)
                //   [2..4]  N_DR of this fragment
                //   [4..]   records
                if payload.len() > 4 {
                    records.extend_from_slice(&payload[4..]);
                }
            }

            // done_byte == 0x00: last/only fragment; non-zero: more to come.
            if done_byte == 0x00 {
                break;
            }
            seq_num = done_byte;
            is_first = false;
        }

        self.last_time = start_time.elapsed().as_secs_f64() * 1000.0;

        Ok(Szl {
            header: SzlHeader { length_dr, n_dr },
            data: records,
        })
    }

    /// ### Reads a raw SZL (System Status List) block from the PLC.
    ///
    /// SZL reads use ROSCTR `0x07` (Userdata), which is a separate protocol path from
    /// normal data-area reads. Multi-fragment responses are handled automatically.
    ///
    /// ### Parameters
    /// - `szl_id`: SZL list identifier (e.g. `S7_SZL_DIAG_BUFFER`, `S7_SZL_CPU_INFO`).
    /// - `szl_index`: SZL index (usually `0` for the complete list).
    ///
    /// ### Returns
    /// `Ok(Szl)` containing the raw record bytes and the parsed header.
    ///
    /// ### Errors
    /// - `S7Error::NotConnected`: client not connected.
    /// - `S7Error::SzlReadFailed`: PLC returned a non-success data return code.
    /// - `S7Error::IsoInvalidTelegram`, `S7Error::IsoInvalidHeader`, `S7Error::IsoFragmentedPacket`: protocol errors.
    /// - `S7Error::Io`: network I/O error.
    ///
    /// ### Suggestion
    /// On low-level errors, disconnect and reconnect before retrying.
    ///
    pub fn read_szl(&mut self, szl_id: u16, szl_index: u16) -> Result<Szl, S7Error> {
        self.read_szl_block(szl_id, szl_index)
    }

    /// ### Reads and decodes the PLC diagnostic buffer (SZL `0x00A0`).
    ///
    /// Returns entries newest-first, as the PLC orders them. Each entry is 20 bytes:
    /// event ID (2), BCD timestamp (8), info bytes (10).
    ///
    /// ### Returns
    /// `Ok(Vec<DiagnosticEntry>)` — decoded diagnostic entries.
    ///
    /// ### Errors
    /// Same as [`read_szl`](S7Client::read_szl).
    ///
    pub fn read_diagnostic_buffer(&mut self) -> Result<Vec<DiagnosticEntry>, S7Error> {
        let szl = self.read_szl_block(S7_SZL_DIAG_BUFFER, 0)?;
        const ENTRY_LEN: usize = 20;
        let mut entries = Vec::new();
        let mut off = 0;
        while off + ENTRY_LEN <= szl.data.len() {
            let rec = &szl.data[off..off + ENTRY_LEN];
            let event_id = make_u16!(rec[0], rec[1]);
            let mut ts_bytes = [0u8; 8];
            ts_bytes.copy_from_slice(&rec[2..10]);
            let timestamp = parse_bcd_timestamp(&ts_bytes);
            let mut info = [0u8; 10];
            info.copy_from_slice(&rec[10..20]);
            entries.push(DiagnosticEntry {
                event_id,
                timestamp,
                info,
            });
            off += ENTRY_LEN;
        }
        Ok(entries)
    }

    /// ### Reads component-identification strings from SZL `0x001C`.
    ///
    /// Returns a [`CpuInfo`] with the module type name, module name, AS name,
    /// copyright string, and serial number, trimmed of null/whitespace padding.
    ///
    /// ### Errors
    /// Same as [`read_szl`](S7Client::read_szl). Also returns `S7Error::SzlReadFailed`
    /// if the returned record length is too short to contain an index field.
    ///
    pub fn read_cpu_info(&mut self) -> Result<CpuInfo, S7Error> {
        let szl = self.read_szl_block(S7_SZL_CPU_INFO, 0)?;
        let rec_len = szl.header.length_dr as usize;
        if rec_len < 4 {
            return Err(S7Error::SzlReadFailed);
        }
        let mut info = CpuInfo::default();
        let mut off = 0;
        while off + rec_len <= szl.data.len() {
            let rec = &szl.data[off..off + rec_len];
            let idx = make_u16!(rec[0], rec[1]);
            let text: String = String::from_utf8_lossy(&rec[2..])
                .chars()
                .take_while(|&c| c != '\0')
                .collect::<String>()
                .trim_end()
                .to_string();
            match idx {
                1 => info.module_type_name = text,
                2 => info.module_name = text,
                3 => info.as_name = text,
                6 => info.copyright = text,
                7 => info.serial_number = text,
                _ => {}
            }
            off += rec_len;
        }
        Ok(info)
    }

    /// ### Reads work memory information from SZL `0x0013`.
    ///
    /// Returns one [`WorkMemoryRecord`] per memory area reported by the PLC.
    /// Typical S7-300/400 PLCs return 3 records (code, data, and retentive areas).
    ///
    /// ### Errors
    /// Same as [`read_szl`](S7Client::read_szl).
    ///
    pub fn read_work_memory(&mut self) -> Result<Vec<WorkMemoryRecord>, S7Error> {
        let szl = self.read_szl_block(S7_SZL_WORK_MEMORY, 0)?;
        let rec_len = szl.header.length_dr as usize;
        let mut records = Vec::new();
        let mut off = 0;
        while off + rec_len <= szl.data.len() {
            let rec = &szl.data[off..off + rec_len];
            if rec.len() >= 12 {
                records.push(WorkMemoryRecord {
                    index:       make_u16!(rec[0], rec[1]),
                    area_type:   make_u16!(rec[2], rec[3]),
                    total_bytes: u32::from_be_bytes([rec[4], rec[5], rec[6], rec[7]]),
                    used_bytes:  u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]),
                });
            }
            off += rec_len;
        }
        Ok(records)
    }

    /// Resolves the connected CPU's family for feature dispatch.
    ///
    /// Prefers the profile recorded by the `connect_*` helper ([`connect_profile`](S7Client::connect_profile)).
    /// Falls back to a best-effort match against the CPU model string from SZL `0x001C`
    /// ([`read_cpu_info`](S7Client::read_cpu_info)) when the profile is unset or ambiguous
    /// (e.g. connected via [`connect_rack_slot`](S7Client::connect_rack_slot)). An explicit
    /// profile always wins over the fallback — VIPA clones and CPs can report misleading
    /// module strings.
    fn resolve_cpu_family(&mut self) -> CpuFamily {
        if let Some(family @ (CpuFamily::S71200_1500 | CpuFamily::S7300)) = self.connect_profile {
            return family;
        }
        if let Ok(info) = self.read_cpu_info() {
            let name = &info.module_type_name;
            if name.contains("1200") || name.contains("1500") {
                return CpuFamily::S71200_1500;
            }
            if name.contains("S7-300") || name.starts_with("CPU 3") {
                return CpuFamily::S7300;
            }
            if name.contains("S7-400") || name.starts_with("CPU 4") {
                return CpuFamily::S7400;
            }
        }
        self.connect_profile.unwrap_or(CpuFamily::Other)
    }

    /// ### Reads scan cycle time statistics.
    ///
    /// Returns a [`CycleTimeInfo`] with the OB1 execution count and the minimum,
    /// maximum, and most-recent cycle times in milliseconds.
    ///
    /// On S7-1200/1500 this reads SZL `0x0194` directly. **S7-300/400 expose no SZL**
    /// containing OB1 cycle-time statistics; instead this attempts the TIS `TIMEMEAS`
    /// userdata subfunction.
    ///
    /// ### ⚠ Experimental on S7-300/400
    /// The TIS path is **unverified against real hardware** — see
    /// `docs/protocol/tis-timemeas.md`. No known open-source S7 client implements
    /// `TIMEMEAS`, and the Wireshark dissector itself has "never seen" one on the wire, so
    /// the request parameters and response layout used here are documented best-effort
    /// guesses. A PLC-level rejection surfaces as a typed error (never a fabricated
    /// result), but a response the PLC *accepts* may still be decoded into a
    /// `CycleTimeInfo` that does not actually represent cycle-time statistics. Treat any
    /// S7-300/400 result as provisional until confirmed against a physical CPU; a PLC-side
    /// DB-publish remains the recommended production approach for these families.
    ///
    /// ### Notes
    /// The PLC must be in RUN mode for cycle time data to be meaningful. In STOP mode
    /// all time fields may be zero.
    ///
    /// ### Errors
    /// On S7-1200/1500: same as [`read_szl`](S7Client::read_szl); returns
    /// `S7Error::IsoInvalidTelegram` if the SZL payload is shorter than 18 bytes.
    /// On S7-300/400: `S7Error::NotConnected` if not connected; `S7Error::SzlReadFailed` if
    /// the PLC returns a non-success data return code for the TIS request;
    /// `S7Error::IsoInvalidTelegram` if the response is malformed or shorter than the
    /// expected minimum length; other low-level errors (`S7Error::Io`, `S7Error::Iso*`) as
    /// for any network operation.
    ///
    pub fn read_cycle_time(&mut self) -> Result<CycleTimeInfo, S7Error> {
        match self.resolve_cpu_family() {
            CpuFamily::S7300 | CpuFamily::S7400 => return self.read_cycle_time_tis(),
            CpuFamily::S71200_1500 | CpuFamily::Other => {}
        }

        let szl = self.read_szl_block(S7_SZL_CYCLE_TIME, 0)?;
        if szl.data.len() < 18 {
            return Err(S7Error::IsoInvalidTelegram);
        }
        let d = &szl.data;
        Ok(CycleTimeInfo {
            ob1_count:  u32::from_be_bytes([d[2], d[3], d[4], d[5]]),
            min_ms:     u32::from_be_bytes([d[6],  d[7],  d[8],  d[9]])  as f64 / 10.0,
            max_ms:     u32::from_be_bytes([d[10], d[11], d[12], d[13]]) as f64 / 10.0,
            current_ms: u32::from_be_bytes([d[14], d[15], d[16], d[17]]) as f64 / 10.0,
        })
    }

    // S7-300/400 branch of read_cycle_time(): experimental TIS TIMEMEAS path. See
    // crate::tis and docs/protocol/tis-timemeas.md.
    fn read_cycle_time_tis(&mut self) -> Result<CycleTimeInfo, S7Error> {
        self.last_time = 0.0;
        self.chunks = 0;

        if !self.connected {
            return Err(S7Error::NotConnected);
        }

        let start_time = Instant::now();
        self.chunks = 1;
        let stream = self.stream.as_mut().unwrap();
        let result = crate::tis::read_cycle_time(stream, self.pdu_length);
        self.last_time = start_time.elapsed().as_secs_f64() * 1000.0;
        result
    }
}

impl Default for S7Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for S7Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
