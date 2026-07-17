# TIS TIMEMEAS — S7-300/400 cycle-time reads

**Status: experimental, unverified against real hardware.** Do not rely on this path for
production monitoring; see [Recommended production alternative](#recommended-production-alternative).

## Why this exists

S7-1200/1500 expose OB1 scan-cycle statistics via SZL `0x0194` (see `S7Client::read_cycle_time`
and `CycleTimeInfo` in `doc/Documentation.md`). **S7-300/400 expose no SZL containing this
data.** STEP 7's "Module Information → Scan Cycle Time" retrieves it from these CPU families
through a different mechanism entirely: the TIS ("Test and Installation") userdata subfunction
`TIMEMEAS`.

No known open-source S7 client (snap7, libnodave, PLC4X, gos7) implements TIMEMEAS. This
document is an original write-up derived from reading two independent references — it is not a
copy or translation of either.

## Sources

- **Wireshark `packet-s7comm.c`** (GPLv2) — the de facto protocol specification for S7comm,
  authored by Thomas Wiens. Used here strictly as documentation: field names, offsets, and
  enum values were read and independently re-described; no dissector code was copied or
  transliterated into Rust7. All line numbers below refer to the file as fetched from
  `github.com/wireshark/wireshark` (`epan/dissectors/packet-s7comm.c`) during this work.
- **Apache PLC4X `s7.mspec`** (Apache-2.0) — `protocols/s7/src/main/resources/protocols/s7/s7.mspec`.
  Used to cross-check the userdata envelope grammar (`S7ParameterUserData`,
  `S7ParameterUserDataItemCPUFunctions`). Framing details derived from it are attributed here
  per Apache-2.0.

## The userdata envelope (fully specified — high confidence)

Every ROSCTR `0x07` (Userdata) request/response, regardless of function group, shares this
envelope. Rust7 implements it once as `build_userdata_request()` / `read_userdata_response()`
in `src/client.rs`, shared by both the existing SZL path and TIMEMEAS.

### Request

| Bytes | Field | Value |
|---|---|---|
| 0–3 | TPKT: version, reserved, total length (BE u16) | `0x03 0x00 hi lo` |
| 4–6 | COTP: length, PDU type, EOT | `0x02 0xF0 0x80` |
| 7 | S7 protocol ID | `0x32` |
| 8 | ROSCTR | `0x07` (Userdata) |
| 9–10 | Redundancy ID | `0x00 0x00` |
| 11–12 | PDU reference (BE u16) | client-chosen, echoed back only |
| 13–14 | Parameter length (BE u16) | `8` (SHORT) or `12` (EXT) |
| 15–16 | Data length (BE u16) | length of the data block that follows |
| 17 | function | `0x00` |
| 18 | item count | `0x01` |
| 19 | item type | `0x12` (`S7ParameterUserDataItemCPUFunctions`, PLC4X `s7.mspec:189-204`) |
| 20 | item length (sub-length) | `4` (SHORT) or `8` (EXT) |
| 21 | **method** | `0x11` = SHORT (single-shot/first); `0x12` = EXT (continuation) |
| 22 | **type\|group** | `(type << 6) \| funcgroup` — see below |
| 23 | subfunction | function-group-specific |
| 24 | sequence number | `0x00` on first request; PLC's echoed value on continuation |
| 25–28 | *(EXT only)* dataUnitRef, lastDataUnit, errorCode(2) | zeroed on request |
| — | data block | function-group-specific (SZL: SZL-ID+INDEX; TIS: see below) |

**method / EXT gating** (`packet-s7comm.c:6761-6763`, `s7comm_decode_ud`): the dissector reads
this byte into a local variable it names `varspec_syntax_id` and compares it against
`S7COMM_SYNTAXID_EXT = 0x12` (`packet-s7comm.c:445`) to decide whether the extra
dataUnitRef/lastDataUnit/errorCode fields follow (`packet-s7comm.c:6855-6868`). Rust7's SZL
builder already relies on exactly this: `build_szl_first_request` uses method `0x11` (no EXT
fields, 8-byte param block); `build_szl_next_request` uses method `0x12` (EXT fields present,
12-byte param block, carrying the echoed sequence number for continuation).

**type\|group byte** (`packet-s7comm.c:6780-6787`): top 2 bits = type (`0` = Indication,
`1` = Request, `2` = Response — inferred from Rust7's existing SZL request byte `0x44` = type
`1`/group `4`), bottom 6 bits = function group. Function groups
(`packet-s7comm.c:672-691`):

| Constant | Value | Name |
|---|---|---|
| `S7COMM_UD_FUNCGROUP_TIS` | `0x01` | **Programmer commands** |
| `S7COMM_UD_FUNCGROUP_CYCLIC` | `0x02` | Cyclic services |
| `S7COMM_UD_FUNCGROUP_BLOCK` | `0x03` | Block functions |
| `S7COMM_UD_FUNCGROUP_CPU` | `0x04` | CPU functions (used by SZL reads) |

So a TIS **request** type|group byte is `(1 << 6) | 0x01 = 0x41`.

> Cross-check against PLC4X: `s7.mspec:189-204`'s `S7ParameterUserDataItemCPUFunctions` models
> this same byte as two 4-bit fields (`cpuFunctionType`, `cpuFunctionGroup`) rather than 2+6
> bits. The two models disagree on bit width, but for the concrete byte values used here
> (`0x44` for SZL, `0x41` for TIS) both derivations land on the same numeric byte — `0x40` from
> "type=1 request" under either split, `| 0x04`/`| 0x01` for the group. Wireshark's split is
> treated as authoritative here since it correctly accommodates function-group values above 15
> (e.g. `S7COMM_UD_FUNCGROUP_NCPRG = 0x3f`, `packet-s7comm.c:679`, which cannot fit in a 4-bit
> field).

### Response

Response envelope offsets (relative to the first byte *after* the 7-byte TPKT/COTP header,
i.e. `response[0]` = S7 protocol ID byte) are validated against the S7comm response structure
and, for the SZL case, against moka7/Snap7 behavior:

| Offset | Field |
|---|---|
| 19 | continuation byte: `0x00` = last/only fragment; non-zero = echo as `seq_num` in the next request |
| 22 | data-block return code (`0xFF` = success) |
| 23 | transport size (unread by Rust7) |
| 24–25 | data-block payload length (BE u16) |
| 26+ | function-group-specific payload |

This preamble (return code, transport size, data length) is parsed identically for **every**
function group by the dissector's shared `s7comm_decode_ud_data` (`packet-s7comm.c:6701-6874`,
specifically the `ret_val`/`tsize`/`len` reads before the `switch (funcgroup)` dispatch at
`packet-s7comm.c:6617-6621`) — it is not SZL-specific. Rust7 implements this shared read as
`read_userdata_response()` in `src/client.rs`, used by both the SZL path and TIMEMEAS.

## TIS TIMEMEAS specifics

TIS subfunctions (`packet-s7comm.c:759-778`):

| Constant | Value | Name |
|---|---|---|
| `S7COMM_UD_SUBF_TIS_BLOCKSTAT` | `0x01` | Block status |
| `S7COMM_UD_SUBF_TIS_VARSTAT` | `0x02` | Variable status |
| ... | | |
| `S7COMM_UD_SUBF_TIS_TIMEMEAS` | **`0x06`** | **"Time measurement from to"** |
| `S7COMM_UD_SUBF_TIS_DISABLEJOB` | `0x0d` | Disable job |
| `S7COMM_UD_SUBF_TIS_ENABLEJOB` | `0x0e` | Enable job |
| `S7COMM_UD_SUBF_TIS_DELETEJOB` | `0x0f` | Delete job |
| `S7COMM_UD_SUBF_TIS_READJOBLIST` | `0x10` | Read job list |
| `S7COMM_UD_SUBF_TIS_READJOB` | `0x11` | Read job |
| `S7COMM_UD_SUBF_TIS_REPLACEJOB` | `0x12` | Replace job |

### Single-shot vs. job — the critical decision

`TIMEMEAS = 0x06` is **not** one of the job-lifecycle subfunctions (`0x0d`–`0x12`, handled by
`s7comm_decode_ud_tis_jobs`, `packet-s7comm.c:4277-4385`). At the framing level it is dispatched
as a direct subfunction like `BLOCKSTAT`/`VARSTAT`, meaning **one request → one response**, not
a job you create/poll/delete. Rust7 implements it single-shot (`USERDATA_METHOD_SHORT`, no
continuation expected).

This is inferred from framing structure, not confirmed against real traffic. The subfunction's
own name — "Time measurement **from** ... **to**" — suggests trigger/window semantics (arm a
measurement between two points, then read the result), which *could* imply an actual PLC-side
job even though it isn't classified with the other job subfunctions in the dissector. If a real
CPU rejects the single-shot form (e.g. with an S7 header-level error, or a non-`0xFF` data
return code), the next step is to try issuing it through `READJOB`/`ENABLEJOB` as a
pseudo-job — **not implemented here**. See `// VERIFY-ON-HARDWARE` markers in `src/tis.rs`.

### Data-part structure (undecoded by the dissector)

The TIS data part is dissected by `s7comm_decode_ud_tis_subfunc` (`packet-s7comm.c:5213-5233`),
which reads a wrapper — `parametersize`(2, BE) + `datasize`(2, BE) — then dispatches a parameter
block (`s7comm_decode_ud_tis_param`, `packet-s7comm.c:4187-4266`) and a data block
(`s7comm_decode_ud_tis_data`, `packet-s7comm.c:5145-5205`) of those sizes.

**Parameter block** (`s7comm_decode_ud_tis_param`): for a *request*, if `parametersize >= 4`,
two generic 16-bit registers ("TIS Parameter 1", "TIS Parameter 2",
`packet-s7comm.c:7502-7510`) are read; larger `parametersize` values unlock more fields
(answer size, trigger event, block/address, call environment — none relevant here since we send
the minimal 4-byte form). For a *response*, exactly two registers ("TIS Result Parameter 1/2",
`packet-s7comm.c:7532-7537`) are always read when `parametersize > 0`.

**Data block** (`s7comm_decode_ud_tis_data`): dispatches on `subfunc`
(`packet-s7comm.c:5160-5202`) — `OUTISTACK`, `OUTBSTACK`, `OUTLSTACK`, `BREAKPOINT`,
`EXITHOLD`, `BLOCKSTAT`/`BLOCKSTAT2`, `VARSTAT`, the job subfunctions, `MODVAR`, `FORCE` all have
dedicated decoders. **`TIMEMEAS` (`0x06`) is absent from this switch** and falls through to the
`default:` arm, which treats the bytes as opaque/unknown
(`hf_s7comm_varstat_unknown`, `packet-s7comm.c:5198-5201`).

Combined with the subfunction-name registration comment `"Time measurement from to"
/* never seen yet */` (`packet-s7comm.c:786`), this means: **no public source describes what
bytes a TIMEMEAS request parameter block should contain to select "OB1 cycle time", nor what
bytes a TIMEMEAS response data block contains.** Everything below this point is a documented
guess.

## Rust7's request (guess — VERIFY-ON-HARDWARE)

```
type|group   = 0x41            (REQ, TIS)
subfunc      = 0x06            (TIMEMEAS)
method       = 0x11            (SHORT — single-shot, no continuation)
seq_num      = 0x00
parametersize = 4
datasize      = 0
parameter block = [0x00, 0x00, 0x00, 0x00]   // TIS Parameter 1 = 0, TIS Parameter 2 = 0
data block       = (empty)
```

`// VERIFY-ON-HARDWARE`: the two zeroed parameter registers are the only well-formed minimal
choice available; their actual meaning for TIMEMEAS (an OB number selector? a measurement
point/trigger reference? unused for this subfunction entirely?) is unknown. If real hardware
rejects this frame, capturing a real STEP 7 "Scan Cycle Time" session with Wireshark is the only
way to determine the correct values.

## Rust7's response parsing (guess — VERIFY-ON-HARDWARE)

The response is parsed generically down to the TIS wrapper (`parametersize`, `datasize`,
`res_param1`, `res_param2` — these positions and widths *are* confirmed by the dissector, see
above) into `tis::TimeMeasRaw`. The response *data block* content
(`TimeMeasRaw::data`) is then — as a hypothesis, not a confirmed fact — mapped onto
`CycleTimeInfo` using the **same byte layout as SZL `0x0194`** on S7-1200/1500 (four big-endian
`u32` fields: `ob1_count`, `min` (0.1 ms units), `max`, `current`; see `read_cycle_time`'s SZL
decode in `src/client.rs`), since that is the only documented Siemens cycle-time byte convention
available anywhere in this codebase's references.

`// VERIFY-ON-HARDWARE`: there is no evidence this layout applies to TIMEMEAS specifically. It
is entirely possible the real response uses different field widths, a different unit, a
different field order, or an entirely different structure (e.g. a single elapsed-time value
rather than min/max/current statistics, matching the "from...to" naming). If the response data
block is shorter than 16 bytes, Rust7 returns `S7Error::IsoInvalidTelegram` rather than
fabricating a `CycleTimeInfo`; if it is 16+ bytes, the returned `CycleTimeInfo` **may still be
wrong** — the length check alone does not confirm the layout.

## Rust7 integration

- `read_cycle_time()` on `S7Client` dispatches on CPU family (see `CLAUDE.md` /
  `doc/Documentation.md`). On `CpuFamily::S7300` / `CpuFamily::S7400`, it now attempts this TIS
  path instead of immediately returning `S7Error::UnsupportedCpuFamily`.
- A non-success data return code, or a response that doesn't parse to the expected minimum
  length, surfaces as a typed error (`S7Error::SzlReadFailed` for a PLC-level rejection reusing
  the existing "non-success userdata return code" semantic, or `S7Error::IsoInvalidTelegram` for
  a malformed/too-short response) — never a silently-fabricated `CycleTimeInfo`.
- Connection type: TIS is a "Programmer commands" function group; STEP 7 issues it over a PG
  connection. `S7Client`'s default `conn_type` is `CT_PG`, so no new configuration is required
  for the common case. If a real CPU rejects TIMEMEAS specifically over an `CT_OP`/`CT_S7`
  connection, that would surface as a PLC-level rejection (see above) — distinguishing "wrong
  connection type" from "TIMEMEAS not supported at all" from the response alone is not currently
  possible and is a `// VERIFY-ON-HARDWARE` gap.

## Recommended production alternative

Until this path is validated against real hardware, the reliable way to monitor S7-300/400 scan
cycle time is a **PLC-side DB-publish**: a small STEP 7 edit (a handful of AWL/SCL lines in OB1,
or reading system data from `OB1_PREV_CYCLE`/`OB1_MIN_CYCLE`/`OB1_MAX_CYCLE` in the OB1 start
info and writing them to a DB) that republishes the CPU's own internal cycle-time counters into
a normal Data Block. Rust7's existing `read_db()` then reads them like any other DB — no new
protocol code needed, and it uses data the CPU already computes for its own OB1 start
info, which is unambiguously correct. This is out of scope for this repository (it requires
editing the PLC program, not the client library) but is the production-safe recommendation
until TIS TIMEMEAS is confirmed.

## On-hardware validation procedure

1. Connect STEP 7 / TIA Portal to the target S7-300/400 CPU and open **Online & Diagnostics →
   Module Information → Scan Cycle Time** (or the classic STEP 7 equivalent).
2. Capture the session with Wireshark (`s7comm` filter) — either passively via a SPAN/mirror
   port between the engineering PC and the PLC, or by running Wireshark on the engineering PC
   itself if permitted.
3. Locate the userdata frame with function group `0x01` (Programmer commands) and subfunction
   `0x06` (TIMEMEAS): `tshark -r capture.pcap -Y "s7comm.param.userdata.funcgroup==1 &&
   s7comm.param.userdata.subfunc==6"` (verify the exact field names against the fetched
   dissector, since it may not decode this subfunction by name).
4. Compare the captured request's parameter block against the guessed
   `[0x00, 0x00, 0x00, 0x00]` in `src/tis.rs`, and the captured response's data block against
   the guessed 16-byte `CycleTimeInfo` layout. Update `src/tis.rs` and this document with the
   confirmed values, remove the `// VERIFY-ON-HARDWARE` markers, and un-ignore the
   corresponding tests.
5. Repeat across at least one S7-300 and one S7-400 CPU with different firmware versions if
   possible — firmware variance across the family is an open risk (see below).

## Open risks

1. **Payload correctness is unproven.** This is the top risk. Everything under "Rust7's
   request" and "Rust7's response parsing" above is a documented guess.
2. **Single-shot vs. job.** If TIMEMEAS actually requires arming via a job subfunction first,
   the single-shot request will likely be rejected outright (a clean, detectable failure) rather
   than silently misinterpreted — but the correct implementation would need the job lifecycle.
3. **Protection level / connection type.** STEP 7 "Programmer commands" functions may require a
   PG connection and/or a CPU protection level that permits online test functions; a real CPU
   configured with write-protection could reject TIMEMEAS regardless of payload correctness.
4. **Firmware variance.** Older S7-300 firmware may not implement TIMEMEAS at all, or may use a
   different payload shape than newer S7-400 firmware.
