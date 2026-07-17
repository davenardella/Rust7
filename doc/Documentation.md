# Rust7 manual
## Background

This project is directly derived from <a href="https://github.com/davenardella/snap7" target="_blank">Snap7</a>. For more information on data formatting (endianess), PLC areas, etc., I recommend referring to the original project documentation, available into /doc folder.

---
## Briefly
Communication with Siemens PLCs via the S7 protocovl occurs according to the classic client-server model, in which the PC is the client and the PLC is the server.

The mechanism is quite simple: to read or write data to a PLC, you must first connect to it and then send a read request, after which you will receive the data, or a write request containing the data to be written.
In both cases, you will receive a response containing the outcome of the operation.

![cli srv](img/cli-srv.png)

### Connection
The S7 protocol is encapsulated in TCP/IP, so the connection must first follow TCP/IP rules, meaning it must contain the IP address and port.
In reality, there are other parameters, such as Rack and Slot or TSAP records, but there are support methods that hide these details.

![s7 encapsulation](img/s7_encapsulation.png)

There are four connection methods available:

* `connect_s71200_1500()` 
* `connect_s7300()` 
* `connect_rack_slot()`
* `connect_tsap()`

The first two are fairly self-explanatory; the third is for use with **S7400**, **WinAC**, and **Sinamics**; the rack and slot parameters will be copied from the project's hardware configuration.
The fourth is for use with **LOGO!** or **S7200** only.
We'll explore these in detail in the API. 

---

### Workflow

**First consideration:**

The connection process is quite complex, and the PLC is not a web server, so the connection must be kept active at all times.

**Second consideration:**

There is no such thing as problem-free communication, and this case is no exception. The library reports two categories of errors: low-level errors such as TCP, corrupt headers due to fragmented packets, etc., and high-level errors, such as a read request from a non-existent database.
In the first case, it is **strongly recommended** to disconnect from the PLC and reconnect before performing any other operations. This behavior is the same as that adopted by WinCC and all other SCADA programs.

**Third consideration:**

Your software will almost certainly have a thread-loop for reading and writing data ciclically, and you'll also have to handle communication errors, such as a physical disconnection of the network cable, without reporting errors, pop-ups, or clogging the system with exceptions to handle.  
But above all, the software must be **pass-through** (non-blocking)

What should we do?

Over the years, this is the question I've been asked most frequently.
So I'll show you a workflow that's the same one we use (not just with Snap7).

First, since low-level error handling involves disconnecting and connecting, these two methods will become part of our loop. When we initialize the program, we'll simply create the Client object and optionally preset the connection (set a port other than 102, set timeouts, etc.).


![workflow](img/workflow.png)

A very simplistic pseudo-code would be this:

```rust
while !exit_request{
    
    if !Client.connected {
        let _ = client.connect_s71200_1500(ip_address);
    }
    
    if Client.connected {
        if !do_read_and_write() { // <-- returns false when severe error occurs
            Client.disconnect();
        }
    }

    thread::sleep(100);
}
```

# API
---

#### Connection Setup


|Prototype|Behaviour|      
|---|---|
|`set_connection_type`|Changes the S7 connection type to the PLC       |
|`set_timeout`        |Sets operations timeout                         |
|`set_connection_port`|Sets the TCP Connection Port                    |

#### Connection
|Prototype|Behaviour|      
|---|---|
|`connect_s71200_1500`|Connects to the S71200 or S71500 families            |
|`connect_s7300`      |Connects to S7300 family                             |
|`connect_rack_slot`  |Connects to a Siemens PLC/Drive using Rack and Slot  |
|`connect_tsap`       |Connects to a Siemens ISO-Hardware using TSAP records|
|`disconnect`         |Closes the connection                                |

#### Raw Read/Write methods
|Prototype|Behaviour|      
|---|---|
|`read_area`     |Reads a block of data from a specific S7 memory area  |
|`write_area`    |Writes a block of data to a specific S7 memory area   |

#### Simplified Read/Write methods
|Prototype|Behaviour|      
|---|---|
|`read_db`       |Reads a block of byte from a specific Data Block (DB) |
|`read_bit`      |Reads a bit from a specific S7 memory area            |
|`write_db`      |Writes a block of byte to a specific Data Block (DB)  |
|`write_bit`     |Writes a bit to a specific S7 memory area             |

#### SZL / Diagnostics methods
|Prototype|Behaviour|
|---|---|
|`read_szl`              |Reads a raw SZL (System Status List) block from the PLC  |
|`read_diagnostic_buffer`|Reads and decodes the PLC diagnostic buffer               |
|`read_cpu_info`         |Reads CPU component-identification strings                |
|`read_work_memory`      |Reads work memory area sizes from SZL `0x0013`            |
|`read_cycle_time`       |Reads OB1 scan cycle time statistics (S7-1200/1500 via SZL `0x0194`; S7-300/400 unsupported for now)|
|`describe_event`        |Maps a diagnostic event ID to a human-readable description|

## Connection setup methods
---

```rust
pub fn set_connection_type(&mut self, connection_type: u16)
```
### Changes the S7 connection type to the PLC

The three possible connection types are:
- `CT_PG`: (as a programming device)
- `CT_OP`: (as an HMI)
- `CT_S7`: (as a generic device)

In practice, there aren't many differences; the CT_PG connection should ensure
better system responsiveness, but in reality, I've never noticed any noticeable differences.

`CT_PG` is used by default.

With very old PLCs (early S7300 series) that have limited communication resources,
the connection may be rejected if we have S7Manager with many online windows open at the same time.  
In this case, use `CT_OP` or `CT_S7`. 
 
#### Parameters
- `connection_type`: Connection type.
 
#### Returns
`Ok(())` on success, or an `S7Error` on failure.

#### Errors
- `S7Error::InvalidFunParameter`: Invalid parameter supplied to the function.

#### Notes
1. The client must not be connected (that is, call this method before connecting).
2. This method is not useful if you use `connect_tsap()` because the connection_type is already contained in the REMOTE_TSAP record.

---

```rust
pub fn set_timeout(&mut self, co_timeout_ms: u64, rd_timeout_ms: u64, wr_timeout_ms: u64 )
```
### Sets operations timeout

#### Parameters
- `co_timeout_ms` : TCP Connection timeout (ms) (Default = 3000 ms)
- `rd_timeout_ms` : Read Connection timeout (ms) (Default = 1000 ms)
- `wr_timeout_ms` : Write Connection timeout (ms) (Default = 500 ms)

#### Returns
`Ok(())` on success, or an `S7Error` on failure.

#### Errors
- `S7Error::InvalidFunParameter`: Invalid parameter supplied to the function.

#### Notes
1. Values must be > 0, otherwise they are ignored
2. The client must not be connected (that is, call this method before connecting).

---
```rust
pub fn set_connection_port(&mut self, port: u16)
```
### Sets the TCP Connection Port
The default S7 Port is 102, but if you need NAT the addresses you can use this method to change the default value.

#### Parameters
- `port`: TCP port number (1–65535). Default is 102.

#### Returns
`Ok(())` on success, or an `S7Error` on failure.

#### Errors
- `S7Error::InvalidFunParameter`: `port` was 0.

#### Notes
1. Value must be > 0, otherwise it is rejected.
2. The client must not be connected (that is, call this method before connecting).

---
## Connection methods
---

```rust
pub fn connect_s71200_1500(&mut self, ip: &str) -> Result<(), S7Error>
```
### Connects to the S71200 or S71500 families   

This helper method is same as `connect_rack_slot()` with rack=0 and slot=0. On success, sets `connect_profile` to `Some(CpuFamily::S71200_1500)` — consulted by family-dispatched methods such as `read_cycle_time()`.
### Parameters
- `ip`  : PLC IPV4 address.
 
For Notes, Return and Errors look at `connect_tsap()`

---
```rust
pub fn connect_s7300(&mut self, ip: &str) -> Result<(), S7Error>
```
### Connects to S7300 family

This helper method is same as `connect_rack_slot()` with rack=0 and slot=2. On success, sets `connect_profile` to `Some(CpuFamily::S7300)` — consulted by family-dispatched methods such as `read_cycle_time()`.
### Parameters
- `ip`  : PLC IPV4 address.
 
For Notes, Return and Errors look at `connect_tsap()`

---
```rust
pub fn connect_rack_slot(&mut self, ip: &str, rack: u16, slot: u16) -> Result<()
```
### Connects to a Siemens PLC/Drive using Rack and Slot
Rack and Slot are Hardware configuration parameters.

For S7300 and S71200/1500 they are fixed, (see `connect_s7300()` and `connect_s71200_1500()` ).
 
Ultimately, you will need of this method only to connect to S7400, WinAC or other Siemens hardware, like Drives, in which Rack and Slot can vary.

On success, sets `connect_profile` to `Some(CpuFamily::Other)`, since the family cannot be inferred from a bare rack/slot pair. Family-dispatched methods such as `read_cycle_time()` fall back to CPU-model-string detection in this case.
 
### Parameters
- `ip` : PLC IPV4 address.
- `rack` : CPU/CU Rack.
- `slot` : CPU/CU Slot.

For Notes, Return and Errors look at `connect_tsap()`

---
```rust
pub fn connect_tsap(&mut self, ip: &str, local_tsap: u16, remote_tsap: u16) -> Result<(), S7Error>
```
### Connects to a Siemens ISO-Hardware using TSAP records

This is the deepest connection method, you will need it only to connect to LOGO! or S7200.

It's internally called by all other connection methods.
#### Parameters
- `ip` : PLC IPV4 address.
- `local_tsap` : Client TSAP.
- `remote_tsap` : Server TSAP (PLC).
 
#### Notes
The connection port used is 102 (S7Protocol Port) unless you
changed it via `set_connection_port()`

#### Returns
`Ok(())` on success, or an `S7Error` on failure.

#### Errors
- `S7Error::TcpConnectionFailed`: TCP connection could not be established.
- `S7Error::IsoConnectionFailed`: ISO connection failed
- `S7Error::PduNegotiationFailed`: PDU negotiation failed.
- `S7Error::Io`: network I/O error.

Example of using TSAPs (from Snap7 manual)
![tsap](img/tsap.png)

---
## Raw Read/Write methods
---

```rust
pub fn read_area(&mut self, area: u8, db_number: u16, start: u16, wordlen: u8, buffer: &mut [u8]) -> Result<(), S7Error>
```
### Reads a block of data from a specific S7 memory area.

#### Parameters
- `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
- `db_number`: DB number (ignored for non-DB areas).
- `start`: Starting element index (byte index for bytes, bit index for bits).
- `wordlen`: Word length constant (e.g., `S7_WL_BYTE`, `S7_WL_BIT`).
- `buffer`: Destination buffer to store the read data.

#### Values
##### area
- `S7_AREA_PE` (0x81): Process Inputs
- `S7_AREA_PA` (0x82): Process Outputs
- `S7_AREA_MK` (0x83): Merkers
- `S7_AREA_DB` (0x84): Data Block
##### wordlen 
- `S7_WL_BIT` (0x01) : Bit access
- `S7_WL_BYTE` (0x02): Byte access
#### Bit access notes
1. The start must be expressed in bits.
For example, if you want to access bit `DBX 45.3`, the start value would be 45 * 8 + 3 = 363.
2. Whatever buffer is passed, only the first byte will be used, which is considered true if !=0 or false if ==0

#### Returns
`Ok(())` Operation succeeded.

#### Errors
##### Low level
- `S7Error::NotConnected`: An attempt was made to read while the client was not connected.
- `S7Error::IsoInvalidHeader`: Invalid ISO Header
- `S7Error::IsoInvalidTelegram`: Inconsistent expected telegram length.
- `S7Error::IsoFragmentedPacket`: ISO Packet fragmented.
- `S7Error::S7Unspecified`: Unknown S7 Error.
- `S7Error::Io`: network I/O error.

##### Suggestion
In case of a low-level error, it is **highly recommended** to disconnect and reconnect the Client (as WinCC or other SCADA do)
  
##### High level
- `S7Error::S7NotFound`: The resource was not found (e.g. a Data Block that does not exist).
- `S7Error::S7InvalidAddress`:
1. Attempt to read beyond the limits.
2. The DB is optimized.

#### Notes
- The number of bytes to read will be equal to the size of the buffer passed.
- Large blocks are automatically split into chunks based on the negotiated PDU size.
- In case of error the buffer contents will be inconsistent and should not be considered.
 
---
```rust
pub fn write_area(&mut self, area: u8, db_number: u16, start: u16, wordlen: u8, data: &[u8]) -> Result<(), S7Error>
```
 
### Writes a block of data to a specific S7 memory area.

#### Parameters
 - `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
 - `db_number`: DB number (ignored for non-DB areas).
 - `start`: Starting element index (byte index for bytes, bit index for bits).
 - `wordlen`: Word length constant (e.g., `S7_WL_BYTE`, `S7_WL_BIT`).
 - `buffer`: Source buffer to write.

#### Values
##### area
- `S7_AREA_PE` (0x81): Process Inputs
- `S7_AREA_PA` (0x82): Process Outputs
- `S7_AREA_MK` (0x83): Merkers
- `S7_AREA_DB` (0x84): Data Block
##### wordlen 
- `S7_WL_BIT` (0x01) : Bit access
- `S7_WL_BYTE` (0x02): Byte access
##### Bit access notes
1. The start must be expressed in bits.
For example, if you want to access bit `DBX 45.3`, the start value would be 45 * 8 + 3 = 363.
2. Whatever buffer is passed, only the first byte will be used, which is considered true if !=0 or false if ==0
3. Writing a bit affects **only that bit**, leaving adjacent bits in the byte unchanged. 

#### Returns
`Ok(())` Operation succeeded.

#### Errors
##### Low level
- `S7Error::NotConnected`: An attempt was made to write while the client was not connected.
- `S7Error::IsoInvalidHeader`: Invalid ISO Header
- `S7Error::IsoInvalidTelegram`: Inconsistent expected telegram length.
- `S7Error::IsoFragmentedPacket`: ISO Packet fragmented.
- `S7Error::S7Unspecified`: Unknown S7 Error.
- `S7Error::Io`: network I/O error.

##### Suggestion
In case of a low-level error, it is **highly recommended** to disconnect and reconnect the Client (as WinCC or other SCADA do)

##### High level
- `S7Error::S7NotFound`: The resource was not found (e.g. a Data Block that does not exist).
- `S7Error::S7InvalidAddress`:
1. Attempt to write beyond the limits.
2. The DB is optimized.

#### Notes
- The number of bytes to write will be equal to the size of the buffer passed.
- Large blocks are automatically split into chunks based on the negotiated PDU size.
- Writing the output buffer (`S7_AREA_PA`) usually does not produce useful results, in fact the output process image will be rewritten by OB1 in the next round

---
## Simplified Read/Write methods
---

```rust
pub fn read_db(&mut self, db_number: u16, start: u16, buffer: &mut [u8]) -> Result<(), S7Error>
```
#### Reads a block of byte from a specific Data Block (DB)

This helper method is same as `read_area()` with:
- area = `S7_AREA_DB`
- wordlen = `S7_WL_BYTE`

#### Parameters
- `db_number`: DB number 
- `start`: Starting byte index 
- `buffer`: Destination buffer to store the read data.

#### Notes
- The number of bytes to read will be equal to the size of the buffer passed.
 
For further info, please refer to `read_area()`

---
```rust
pub fn write_db(&mut self, db_number: u16, start: u16, buffer: &[u8]) -> Result<(), S7Error>
```
#### Writes a block of byte to a specific Data Block (DB)

This helper method is same as `write_area()` with:
- area = `S7_AREA_DB`
- wordlen = `S7_WL_BYTE`

#### Parameters
- `db_number`: DB number 
- `start`: Starting byte index 
- `buffer`: Source buffer to write.

#### Notes
- The number of bytes to write will be equal to the size of the buffer passed.
 
For further info, please refer to `write_area()`

---
```rust
pub fn read_bit(&mut self, area: u8, db_number: u16, byte_num: u16, bit_idx: u8) -> Result<bool, S7Error>
```
 #### Reads a bit from a specific S7 memory area

 This helper method is same as `read_area()` with:
 - wordlen = `S7_WL_BIT`
 - start = `byte_num * 8 + bit_idx`
 
 #### Parameters
 - `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
 - `db_number`: DB number (ignored for non-DB areas).
 - `byte_num`: Byte Number. 
 - `bit_idx`: Bit index inside the byte (0..7).
 
 #### Example
 To read DB10.DBX71.4 use:
 
 ```my_bit = read_bit(S7_AREA_DB, 10, 71, 4);```
 
 #### Returns
 `Ok(<bool>)` or `Err(<S7Error>)`

#### Errors
- `S7Error::InvalidFunParameter`: Invalid parameter supplied to the function.
- Other reported by read_area()

 #### Suggestion
 - Even reading a single bit requires an entire telegram.
 Since reading is non-invasive, if you need to read multiple bits 
 (more or less adjacent in the same area), I recommend reading blocks 
 of bytes and then unpacking them.
 
 For further info, please refer to `read_area()`

---
```rust
pub fn write_bit(&mut self, area: u8, db_number: u16, byte_num: u16, bit_idx: u8, value: bool) -> Result<(), S7Error>
```
#### Writes a bit to a specific S7 memory area

This helper method is same as `write_area()` with:
- wordlen = `S7_WL_BIT`
- start = `byte_num * 8 + bit_idx`

#### Parameters
- `area`: S7 memory area constant (e.g., `S7_AREA_PE`, `S7_AREA_PA`, `S7_AREA_DB`, `S7_AREA_MK`).
- `db_number`: DB number (ignored for non-DB areas).
- `byte_num`: Byte Number. 
- `bit_idx`: Bit index inside the byte (0..7).
- `value`: Value to write (true | false).

#### Example
To write **1** into DB10.DBX71.4 use:

```write_bit(S7_AREA_DB, 10, 71, 4, true);```

#### Returns
`Ok(())` Operation succeeded.

#### Errors
- `S7Error::InvalidFunParameter`: Invalid parameter supplied to the function.
- Other reported by write_area()

#### Notes
- Writing a bit affects only that bit, leaving adjacent bits in the byte unchanged. 

For further info, please refer to `write_area()`
 
# Fields
---

```rust
pub pdu_length: u16
```
#### PDU size negotiated with the PLC during connection setup.
Typical value is 480 bytes. Larger values mean fewer round-trips for large reads/writes.

---
```rust
pub connected: bool
```
#### `true` while a TCP connection to the PLC is active.
#### Note
- This is a cached flag, not a live network probe. If the cable is unplugged, `connected` stays `true` until the next read or write call fails.

---
```rust
pub last_time: f64
```
#### Duration of the most recent operation in milliseconds.
#### Note
- Set to `0.0` when an error occurs before the operation completes.

---
```rust
pub chunks: usize
```
#### Number of PDU round-trips used by the most recent read or write.
Values above 1 mean the data was split across multiple S7 PDUs (auto-chunking).
#### Note
- Set to 0 when an error occurs.

---
# SZL / Diagnostics
---

SZL (System Status List / Systemzustandsliste) reads use the S7 Userdata protocol (ROSCTR `0x07`), a different path from normal data reads. Multi-fragment responses are handled automatically.

## SZL constants

| Constant | Value | Description |
|---|---|---|
| `S7_SZL_CPU_ID`      | `0x0011` | Module identification (order number, firmware version, PLC type) |
| `S7_SZL_WORK_MEMORY` | `0x0013` | Work memory information — total and used bytes per area |
| `S7_SZL_CPU_INFO`    | `0x001C` | Component identification (module name, serial number, AS name, copyright) |
| `S7_SZL_DIAG_BUFFER` | `0x00A0` | Diagnostic buffer — the primary diagnostic facility accessible from outside the PLC |
| `S7_SZL_CYCLE_TIME`  | `0x0194` | Cycle time statistics — OB1 min, max, and current scan cycle times |
| `S7_SZL_CPU_STATUS`  | `0x0424` | Current CPU operating mode (RUN / STOP / STARTUP) |

## SZL types

### `SzlHeader`

```rust
pub struct SzlHeader {
    pub length_dr: u16,  // byte length of each data record
    pub n_dr: u16,       // total number of data records (across all fragments)
}
```

### `Szl`

```rust
pub struct Szl {
    pub header: SzlHeader,
    pub data: Vec<u8>,   // concatenated record bytes; length = header.length_dr * header.n_dr
}
```

### `S7DateTime`

Decoded Siemens `DATE_AND_TIME` (8-byte BCD timestamp, S7 type `DT`).

```rust
pub struct S7DateTime {
    pub year: u16,        // full year (e.g. 2024); BCD 00–89 → 2000–2089, 90–99 → 1990–1999
    pub month: u8,        // 1–12
    pub day: u8,          // 1–31
    pub hour: u8,         // 0–23
    pub minute: u8,       // 0–59
    pub second: u8,       // 0–59
    pub millisecond: u16, // 0–999
    pub weekday: u8,      // Siemens: 1 = Sunday … 7 = Saturday
}
```

### `DiagnosticEntry`

One entry from the PLC diagnostic buffer. Entries are returned newest-first.

```rust
pub struct DiagnosticEntry {
    pub event_id: u16,              // raw event ID; pass to describe_event() for a friendly name
    pub timestamp: Option<S7DateTime>, // None if any BCD nibble was invalid
    pub info: [u8; 10],             // remaining event-specific bytes (not decoded by this library)
}
```

### `CpuFamily`

CPU family recorded by a `connect_*` helper, used to select the correct mechanism for family-specific features (e.g. cycle-time reads via `read_cycle_time()`).

```rust
pub enum CpuFamily {
    S71200_1500, // connected via connect_s71200_1500()
    S7300,       // connected via connect_s7300()
    S7400,       // never set automatically (no dedicated connect_s7400() helper);
                 // only produced by CPU-model-string fallback detection
    Other,       // family not distinguishable (e.g. via connect_rack_slot() / connect_tsap())
}
```

`S7Client::connect_profile: Option<CpuFamily>` holds the value recorded for the current connection (`None` before any `connect_*` call).

### `CpuInfo`

```rust
pub struct CpuInfo {
    pub module_type_name: String, // e.g. "CPU 1516-3 PN/DP"
    pub module_name: String,      // user-assigned name from TIA Portal
    pub as_name: String,          // automation station (project) name
    pub copyright: String,        // Siemens copyright string from firmware
    pub serial_number: String,    // unique hardware serial number
}
```

### `WorkMemoryRecord`

One memory area record from SZL `0x0013`. Byte layout is from the Siemens S7 System and Standard Functions reference manual; validate with a Wireshark capture when changing CPU families.

```rust
pub struct WorkMemoryRecord {
    pub index: u16,       // area identifier as reported by the PLC
    pub area_type: u16,   // area type code; encoding is CPU-family specific
    pub total_bytes: u32, // total size of this memory area in bytes
    pub used_bytes: u32,  // currently used bytes
}
```

### `CycleTimeInfo`

Scan cycle statistics from SZL `0x0194`. All times are in milliseconds (raw PLC unit is 0.1 ms).

```rust
pub struct CycleTimeInfo {
    pub ob1_count: u32,   // OB1 executions since last CPU startup
    pub min_ms: f64,      // minimum scan cycle time in ms (since last startup)
    pub max_ms: f64,      // maximum scan cycle time in ms (since last startup)
    pub current_ms: f64,  // most recently completed scan cycle time in ms
}
```

### `DiagEventInfo`

Returned by `describe_event()`.

```rust
pub struct DiagEventInfo {
    pub class: &'static str,       // event class from the high nibble, e.g. "Mode transitions"
    pub name: Option<&'static str>,// specific event name, or None if not in the lookup tables
}
```

## SZL methods

---
```rust
pub fn read_szl(&mut self, szl_id: u16, szl_index: u16) -> Result<Szl, S7Error>
```
#### Reads a raw SZL block from the PLC.

#### Parameters
- `szl_id`: SZL list identifier (e.g. `S7_SZL_DIAG_BUFFER`, `S7_SZL_CPU_INFO`).
- `szl_index`: SZL index (use `0` for the complete list).

#### Returns
`Ok(Szl)` with raw record bytes and a parsed header.

#### Errors
- `S7Error::NotConnected`: client is not connected.
- `S7Error::SzlReadFailed`: PLC returned a non-success data return code.
- `S7Error::IsoInvalidTelegram`, `S7Error::IsoInvalidHeader`, `S7Error::IsoFragmentedPacket`: protocol errors.
- `S7Error::Io`: network I/O error.

---
```rust
pub fn read_diagnostic_buffer(&mut self) -> Result<Vec<DiagnosticEntry>, S7Error>
```
#### Reads and decodes the PLC diagnostic buffer (SZL `0x00A0`).

Returns entries newest-first. Pass `entry.event_id` to `describe_event()` for a human-readable description.

#### Returns
`Ok(Vec<DiagnosticEntry>)` — may be empty if the buffer contains no entries.

#### Errors
Same as `read_szl()`.

---
```rust
pub fn read_cpu_info(&mut self) -> Result<CpuInfo, S7Error>
```
#### Reads component-identification strings from SZL `0x001C`.

#### Returns
`Ok(CpuInfo)` with module type, name, AS name, copyright, and serial number.

#### Errors
Same as `read_szl()`.

---
```rust
pub fn read_work_memory(&mut self) -> Result<Vec<WorkMemoryRecord>, S7Error>
```
#### Reads work memory information from SZL `0x0013`.

Returns one `WorkMemoryRecord` per memory area reported by the PLC. Typical S7-300/400 PLCs return 3 records (code, data, and retentive areas).

#### Returns
`Ok(Vec<WorkMemoryRecord>)` — one entry per memory area.

#### Errors
Same as `read_szl()`.

---
```rust
pub fn read_cycle_time(&mut self) -> Result<CycleTimeInfo, S7Error>
```
#### Reads scan cycle time statistics.

Returns a `CycleTimeInfo` with the OB1 execution count and the minimum, maximum, and most-recent cycle times in milliseconds.

On S7-1200/1500 this reads SZL `0x0194` directly. **S7-300/400 expose no SZL containing OB1 cycle-time statistics** — the method dispatches on the connected CPU family (see below) and returns `S7Error::UnsupportedCpuFamily` on those families instead of attempting the SZL read.

#### CPU family detection
The family is resolved in this order:
1. The profile recorded by the `connect_*` helper used to connect (`connect_s71200_1500` → 1200/1500, `connect_s7300` → 300). This always wins.
2. If the profile is unset or ambiguous (e.g. connected via `connect_rack_slot`/`connect_tsap`), a best-effort fallback matches the CPU model string from `read_cpu_info()` (SZL `0x001C`) against `"1200"`/`"1500"`/`"S7-300"`/`"S7-400"`/`"CPU 3…"`/`"CPU 4…"` patterns.
3. If neither resolves the family, the pre-existing SZL `0x0194` behavior is used (unchanged for existing callers).

#### Notes
- The PLC must be in RUN mode for meaningful data. In STOP mode all time fields may be zero.
- `fbarresi/softplc` does not support SZL `0x0194`.

#### Returns
`Ok(CycleTimeInfo)`.

#### Errors
Same as `read_szl()`.
Returns `S7Error::IsoInvalidTelegram` if the SZL payload is shorter than 18 bytes.
Returns `S7Error::UnsupportedCpuFamily` on S7-300/400: SZL `0x0194` is S7-1200/1500 only. Use a PLC-side DB-publish for cycle-time monitoring on S7-300/400 until a TIS TIMEMEAS-based path is available.

---
## Diagnostic event ID lookup

```rust
pub fn describe_event(event_id: u16) -> DiagEventInfo
```

Maps a raw `event_id` from a `DiagnosticEntry` to a human-readable class and name.

The lookup follows the same dispatch logic as the Wireshark S7Comm dissector:
- The high nibble (bits 12–15) identifies the event class (e.g. `0x4` → "Mode transitions").
- IDs with class `0x8` or `0x9` are looked up in the standardised diagnostic / predefined-user-event table; all others use the fixed-event table.
- 557 entries are covered in total.

#### Example

```rust
use rust7::{describe_event, S7Client};

let info = describe_event(0x4302);
println!("{}: {}", info.class, info.name.unwrap_or("(unknown)"));
// Mode transitions: Mode transition from STARTUP to RUN
```
