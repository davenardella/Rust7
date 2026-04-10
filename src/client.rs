// Rust7 - Native Rust S7 client (Snap7‑style) for Siemens PLCs.
// Copyright 2025 - Davide Nardella

use std::net::{TcpStream, ToSocketAddrs};
use std::net::Shutdown;
use std::time::Duration;
use std::fmt;
use std::io;
use std::io::{Read, Write};
use std::time::Instant;


// Connection types
pub const CT_PG: u16 = 0x0001; // As PG (Default)
pub const CT_OP: u16 = 0x0002; // As OP 
pub const CT_S7: u16 = 0x0003; // AS S7 Basic

// Areas
pub const S7_AREA_PE: u8 = 0x81;  // Process Inputs
pub const S7_AREA_PA: u8 = 0x82;  // Process Outputs
pub const S7_AREA_MK: u8 = 0x83;  // Merkers
pub const S7_AREA_DB: u8 = 0x84;  // Data Block

// Wordlen
pub const S7_WL_BIT: u8 = 0x01;
pub const S7_WL_BYTE: u8 = 0x02;

// Transport
const TS_RES_BIT: u8 = 0x03;
const TS_RES_BYTE: u8 = 0x04;

// PDU related
const TPKT_ISO_LEN: usize   = 7; // ISO Header length
const PDU_LEN_REQ: u16      = 480; // PDU Length requested for negotiation
const ISO_CR_LEN: usize     = 22;   // Connection request telegram size 
const ISO_CONN_REQ: u8      = 0xE0; // ISO connection requesr
const ISO_CONN_OK: u8       = 0xD0; // ISO connection accepted
const ISO_PN_REQ_LEN: usize = 25;   // PDU negotiation request telegram size 
const ISO_PN_RES_LEN: usize = 27;   // PDU negotiation response telegram size 
const ISO_ID: u8            = 0x03; // RFC 1006 ID
const S7_ID: u8             = 0x32; // S7 Protocol ID


const READ_REQ_LEN: usize   = 31; // TKPT + ISO + S7 headers
const READ_RES_LEN: usize   = 18; // Read job response header length
const WRITE_RES_LEN: usize  = 15; // Write job response header length

const EOT: u8               = 0x80; // ISO End of Trasmission
const RW_RES_OFFSET: usize  = 14;

/// Operation successful
const RES_SUCCESS: u8         = 0xFF; 
/// Invalid Address requested
/// - Trying to read beyond the limits
/// - The DB is optimizad
const RES_INVALID_ADDRESS: u8 = 0x05;  
/// Resource not found
/// - The DB doesn't exists in the CPU
const RES_NOT_FOUND: u8       = 0x0A; 

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

#[derive(Debug)]
pub enum S7Error {
    Io(io::Error),
    NotConnected,
    TcpConnectionFailed,
    ConnectionClosed,
    IsoConnectionFailed,
    IsoFragmentedPacket,
    IsoInvalidHeader,
    IsoInvalidTelegram,
    PduNegotiationFailed,
    InvalidFunParameter,
    S7NotFound,
    S7InvalidAddress,
    S7Unspecified,
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
            S7Error::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for S7Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for S7Error {
    fn from(err: io::Error) -> S7Error {
        S7Error::Io(err)
    }
}
pub struct S7Client {
    stream: Option<TcpStream>,
    port: u16,
    co_timeout_ms: u64,
    rd_timeout_ms: u64,
    wr_timeout_ms: u64,
    conn_type: u16,
    max_rd_pdu_data: u16, // Max Read PDU Payload
    max_wr_pdu_data: u16, // Max Write PDU Payload
    /// PDU length negotiated by the CPU
    pub pdu_length: u16,  
    /// Client connected
    pub connected: bool,
    /// ### Last Job time (ms).
    /// 
    /// If an error occurred the value will be 0
    pub last_time: f64,
    /// ### Indicates how many pieces the data to be read or written in the last operation was divided into
    /// Maybe you need to know it only for extreme tuning
    pub chunks:  usize,
}

    /// ### Checks the incoming ISO Packet coherence
    ///
    /// Typically, a PLC never sends incorrect values, but we may find data in the buffer 
    /// from a fragmented transmission, so it is good practice to check.
    /// 
    fn check_iso_packet(pdu_length: u16, iso_packet: &mut [u8; TPKT_ISO_LEN]) -> Result<usize, S7Error> {
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
        
        if telegram_length < TPKT_ISO_LEN || 
           telegram_length - TPKT_ISO_LEN > pdu_length as usize || 
           telegram_length - TPKT_ISO_LEN == 0 {
            return Err(S7Error::IsoInvalidTelegram);
        }  
        
        // Returns the ramaining byte to read from the telegram
        Ok(telegram_length - TPKT_ISO_LEN)
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
            chunks:0,
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
        
        if connection_type < CT_PG || connection_type > CT_S7 {
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
    pub fn set_timeout(&mut self, co_timeout_ms: u64, rd_timeout_ms: u64, wr_timeout_ms: u64 ) -> Result<(), S7Error> {

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
    pub fn set_connection_port(&mut self, port: u16)  -> Result<(), S7Error> {

        if port == 0 {
            return Err(S7Error::InvalidFunParameter);
        }

        self.port = port;

        Ok(())
    }

    /// ### Connects to the S71200 or S71500 families
    ///
    /// This helper method is same as `connect_rack_slot()` with rack=0 and slot=0
    /// ### Parameters
    /// - `ip`  : PLC IPV4 address.
    /// 
    /// ---
    /// For Notes, Return and Errors look at `connect_tsap()`
    ///
    pub fn connect_s71200_1500(&mut self, ip: &str) -> Result<(), S7Error> {
        self.connect_rack_slot(ip, 0, 0)  
    }

    /// ### Connects to the S7300 family
    /// 
    /// This helper method is same as `connect_rack_slot()` with rack=0 and slot=2
    /// ### Parameters
    /// - `ip`  : PLC IPV4 address.
    /// 
    /// ---
    /// For Notes, Return and Errors look at `connect_tsap()`
    /// 
    pub fn connect_s7300(&mut self, ip: &str) -> Result<(), S7Error> {
        self.connect_rack_slot(ip, 0, 2)
    }

    /// ### Connects to a Siemens PLC/Drive using Rack and Slot
    ///
    /// Rack and Slot are Hardware configuration parameters.
    ///
    /// For S7300 and S71200/1500 they are fixed, (see `connect_s7300()` and `connect_s71200_1500()` ).
    /// 
    /// Ultimately, you will need of this method only to connect to S7400, WinAC or other Siemens 
    /// hardware, like Drives, which Rack and Slot can vary.
    /// 
    /// ### Parameters
    /// - `ip` : PLC IPV4 address.
    /// - `rack` : CPU/CU Rack.
    /// - `slot` : CPU/CU Slot.
    /// 
    /// ---
    /// For Notes, Return and Errors look at `connect_tsap()`
    /// 
    pub fn connect_rack_slot(&mut self, ip: &str, rack: u16, slot: u16) -> Result<(), S7Error> {

        let local_tsap: u16 = 0x0100;
        let remote_tsap: u16 = (self.conn_type << 8) + (rack * 0x20) + slot;        
        self.connect_tsap(ip, local_tsap, remote_tsap)
    }

    /// ### Connects to a Siemens ISO-Hardware using TSAP records
    ///
    /// This is the deepest connection method, you will need it only to connect to LOGO! or S7200.
    /// It's internally called by all other connection methods.
    /// 
    /// ### Parameters
    /// - `ip` : PLC IPV4 address.
    /// - `local_tsap` : Client TSAP.
    /// - `remote_tsap` : Server TSAP (PLC).
    /// 
    /// ### Notes
    ///     The connection port used is 102 (S7Protocol Port) unless you
    ///     changed it via set_connection_port()
    ///
    /// ### Returns
    /// `Ok(())` on success, or an `S7Error` on failure.
    ///
    /// ### Errors
    /// - `S7Error::TcpConnectionFailed`: TCP connection could not be established.
    /// - `S7Error::IsoConnectionFailed`: ISO connection failed
    /// - `S7Error::PduNegotiationFailed`: PDU negotiation failed.
    /// - `S7Error::Io`: network I/O error.
    /// 
    pub fn connect_tsap(&mut self, ip: &str, local_tsap: u16, remote_tsap: u16) -> Result<(), S7Error> {
   
        self.connected = false;
        self.last_time = 0.0;
        let start_time = Instant::now();      
        
        let addr = format!("{}:{}", ip, self.port);
        let co_timeout = Duration::from_millis(self.co_timeout_ms);
        let rd_timeout = Duration::from_millis(self.rd_timeout_ms);
        let wr_timeout = Duration::from_millis(self.wr_timeout_ms);

        let mut stream = TcpStream::connect_timeout(&addr.to_socket_addrs()?.next().ok_or(S7Error::TcpConnectionFailed)?, co_timeout)?;
        
        stream.set_read_timeout(Some(rd_timeout))?;
        stream.set_write_timeout(Some(wr_timeout))?;
        stream.set_nodelay(true)?;
        

        // ISO-on-TCP handshake
        let iso_cr: [u8; ISO_CR_LEN] = [
		    // TPKT (RFC1006 Header)
            ISO_ID, // RFC 1006 ID (3) 
            0x00,   // Reserved, always 0
            hi_part!(ISO_CR_LEN), // High part of packet lenght (entire frame, payload and TPDU included)
            lo_part!(ISO_CR_LEN), // Low part of packet lenght (entire frame, payload and TPDU included)
            // COTP (ISO 8073 Header)
            0x11, // PDU Size Length
            ISO_CONN_REQ, // CR - Connection Request ID
            0x00, // Dst Reference HI
            0x00, // Dst Reference LO
            0x00, // Src Reference HI
            0x01, // Src Reference LO
            0x00, // Class + Options Flags
            0xC0, // PDU Max Length ID
            0x01, // PDU Max Length HI
            0x0A, // PDU Max Length LO
            0xC1, // Src TSAP Identifier
            0x02, // Src TSAP Length (2 bytes)
            hi_part!(local_tsap), // Loc TSAP HI 
            lo_part!(local_tsap), // Loc TSAP LO 
            0xC2, // Rem TSAP Identifier
            0x02, // Rem TSAP Length (2 bytes)
            hi_part!(remote_tsap), // Rem TSAP HI 
            lo_part!(remote_tsap)  // Rem TSAP LO 
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
            0x00, 0x19, 
            0x02, 0xf0, 0x80, 
            S7_ID, 0x01, 0x00, 0x00, 0x04, 0x00, 0x00, 0x08, 0x00, 
            0x00, 0xf0, 0x00, 0x00, 0x01, 0x00, 0x01, 
            hi_part!(PDU_LEN_REQ),
            lo_part!(PDU_LEN_REQ)
        ];
        stream.write_all(&s7_pn)?;
        let mut pn_resp = [0u8; ISO_PN_RES_LEN];
        
        let size_pn = stream.read(&mut pn_resp)?;
        
        if size_pn < ISO_PN_RES_LEN || pn_resp[0] != ISO_ID || pn_resp[7] != S7_ID || pn_resp[17] != 0x00 {
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

    /// ### Closes the connection.
    ///
    /// Safe to call even if the client is not currently connected.
    /// After disconnection, calls to read/write will return `S7Error::NotConnected`.
    /// 
    /// ### Notes
    ///     A Client should be disconnected on low-level error (see read_area() and write_area() suggestion)
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
    pub fn read_area(&mut self, area: u8, db_number: u16, start: u16, wordlen: u8, buffer: &mut [u8]) -> Result<(), S7Error> {

        self.last_time = 0.0;
        self.chunks = 0;

        // Check Area
        let _ = self.check_area(area)?;

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
            self.chunks+=1;

            // Read Request Header
            let mut request: [u8; READ_REQ_LEN] = [ 
                ISO_ID, 0x00,         // RFC 1006 ID (constant)                   0
                0x00, 0x1f,           // Telegram Length (31)                     2
                0x02, 0xf0, 0x80,     // COPT (constant)                          4
                S7_ID,                // S7 Protocol ID                           7
                0x01,                 // Job Type (Data)                          8
                0x00, 0x00,           // Redundancy identification                9
                0x05, 0x00,           // PDU Reference                            11  
                0x00, 0x0e,           // Parameters Length (HI,LO) = 14           13 
                0x00, 0x00,           // No write Payload here : 0                15
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
                0x00, 0x00, 0x00      // 24 bit Address (see below)               28
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
                    _ => return Err(S7Error::S7Unspecified)
                }
            }
          
            // Copy payload
            let payload = &response[READ_RES_LEN..READ_RES_LEN + (size_resp - READ_RES_LEN).min(chunk_size as usize)];
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
    /// will be rewritten by OB1 in the next round
    /// 
    pub fn write_area(&mut self, area: u8, db_number: u16, start: u16, wordlen: u8, buffer: &[u8]) -> Result<(), S7Error> {

        self.last_time = 0.0;
        self.chunks = 0;

        // Check Area
        let _ = self.check_area(area)?;

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
        
        let transport: u8 = if wordlen == S7_WL_BIT { TS_RES_BIT } else { TS_RES_BYTE };

        while offset < datasize{
            self.chunks+=1;
            let chunk_size = (datasize - offset).min(self.max_wr_pdu_data as usize);
            let chunk = &buffer[offset..offset + chunk_size];

            let bits_payload: u16 = if wordlen == S7_WL_BIT { 1 } else { (chunk_size << 3) as u16 };

            // 35 byte Write Request Header
            let mut request = vec![ 
                ISO_ID, 0x00,            // RFC 1006 ID (constant)
                0x00, 0x00,              // Telegram Length (HI,LO) = Payload Size + 35
                0x02, 0xf0, 0x80,        // COPT (constant)
                S7_ID,                   // S7 Protocol ID 
                0x01,                    // Job Type (Data)
                0x00, 0x00,              // Redundancy identification 
                0x05, 0x00,              // PDU Reference
                0x00, 0x0e,              // Parameters Length (HI,LO) = 14
                hi_part!(chunk_size + 4),// HI (Payload Size + 4) 
                lo_part!(chunk_size + 4),// LO (Payload Size + 4)
                0x05,                    // Function: 4 Read Var, 5 Write Var 
                0x01,                    // Items count (used for multivar R/W)
                0x12,                    // Var spec.
                0x0a,                    // constant 0x0a
                0x10,                    // Syntax ID 
                wordlen,
                hi_part!(chunk_size),    // HI Payload size
                lo_part!(chunk_size),    // LO Payload size               
                hi_part!(db_number),     // HI DB Number 
                lo_part!(db_number),     // LO DB Number               
                area,                    // Area ID
                0x00, 0x00, 0x00,        // 24 bit Address (see below)
                0x00,                    // Reserved
                transport,               // TS_RES_BIT or TS_RES_BYTE
                hi_part!(bits_payload),  // HI Payload size (bits) 
                lo_part!(bits_payload)   // LO Payload size (bits)
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
                    _ => return Err(S7Error::S7Unspecified)
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
    pub fn read_db(&mut self, db_number: u16, start: u16, buffer: &mut [u8]) -> Result<(), S7Error> {
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
    ///     Even reading a single bit requires an entire telegram.
    ///     Since reading is non-invasive, if you need to read multiple bits 
    ///     (more or less adjacent in the same area), I recommend reading blocks 
    ///     of bytes and then unpacking them.
    /// ---
    /// For further info, please refer to `read_area()`
    /// 
    pub fn read_bit(&mut self, area: u8, db_number: u16, byte_num: u16, bit_idx: u8) -> Result<bool, S7Error> {

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
    ///     Writing a bit affects only that bit, leaving adjacent bits in the byte unchanged. 
    /// ---
    /// For further info, please refer to `write_area()`
    /// 
   pub fn write_bit(&mut self, area: u8, db_number: u16, byte_num: u16, bit_idx: u8, value: bool) -> Result<(), S7Error> {
        
        if bit_idx > 7 { 
            return Err(S7Error::InvalidFunParameter); 
        }
  
        let start: u16 = byte_num * 8 + bit_idx as u16;
        let mut data = [0u8; 1];
        data[0] = value as u8;        
              
        self.write_area(area, db_number, start, S7_WL_BIT, &mut data)
    }
}

impl Drop for S7Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}