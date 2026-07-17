# rust7 Development Guide

## Table of Contents

- [High Level Overview](#high-level-overview)
- [Quick Start](#quick-start)
- [Commands](#commands)
- [Testing](#testing)
- [Codebase Navigation](#codebase-navigation)
- [Development Workflows](#development-workflows)
- [S7 Protocol Domain Knowledge](#s7-protocol-domain-knowledge)
- [Constraints and Gotchas](#constraints-and-gotchas)
- [Anti-Patterns](#anti-patterns)
- [Troubleshooting](#troubleshooting)

---

## High Level Overview

`rust7` is a pure-Rust, zero-external-dependency, synchronous S7 protocol client for Siemens PLCs. It is the spiritual successor to the Snap7 C++ reference implementation — same API shape, same mental model, 100% safe Rust.

The entire library is two source files. `src/client.rs` contains the complete protocol stack: TPKT (RFC 1006) → ISO 8073 / COTP → S7Comm. `src/lib.rs` re-exports the public surface and nothing else.

**Hard architectural constraints — do not violate:**
- `#![forbid(unsafe_code)]` is enforced at compile time.
- `[dependencies]` is empty. Zero external crates.
- Synchronous only (`std::net::TcpStream`). No async, no tokio.

Full API documentation is in `doc/Documentation.md`. Read it before asking questions about method semantics — this file does not repeat it.

---

## Quick Start

```bash
cargo build
cargo check   # faster iteration
cargo test
```

The `examples/docker/` directory is a **separate Cargo workspace** with its own lockfile. `cargo build` from the project root builds only the library. To build or run the Docker example, `cd examples/docker/` first.

---

## Commands

### Build and Check

```bash
cargo build
cargo build --release
cargo check
cargo check --tests
```

### Lint and Format

```bash
cargo fmt
cargo fmt --check
cargo clippy
cargo clippy --fix  # auto-applies machine-applicable suggestions; commit the result
```

Use `cargo check` (not `cargo build`) for fast incremental correctness checks during iteration.
Run `cargo clippy --fix` after each feature commit; if it makes changes, create a follow-up `chore:` commit.

### Documentation

```bash
cargo doc --no-deps --open
```

### Debugging in Zed

A sample launch configuration lives at `.zed/debug.json.sample`. It covers three targets:

| Label | Binary | Default args |
|---|---|---|
| `diagnostics: localhost` | `examples/diagnostics/target/debug/diagnostics` | `127.0.0.1` |
| `diagnostics: custom PLC IP` | same | `192.168.0.100` (edit to taste) |
| `docker example: SoftPLC container` | `examples/docker/target/debug/docker` | — |

To use: copy `.zed/debug.json.sample` → `.zed/debug.json` (gitignored), then press `F4` or run `debugger: start`. Requires the **CodeLLDB** Zed extension.

### Docker Example (from `examples/docker/`)

```bash
# Start the virtual PLC
docker compose up -d

# Provision DB2 (required before running the example)
curl -X POST 'http://localhost:8080/api/DataBlocks?id=2&size=1024' -H 'accept: */*' -d ''

# Run example
task run
# or without taskfile:
cargo run

docker compose down
```

Requires the `task` CLI (https://taskfile.dev). SoftPLC image: `fbarresi/softplc:latest-linux`. Management UI + Swagger: `http://localhost:8080`. Port 102 = S7 protocol. DB1 exists by default ("Hello, S7!" in first 16 bytes). DB2 must be provisioned via REST before the example runs.

---

## Testing

```bash
# Unit tests (156 tests, no Docker required)
cargo test --lib

# Integration tests (10 tests + 11 ignored probe/VERIFY-ON-HARDWARE tests; requires Docker or Podman)
cargo test --test integration

# Full suite (unit + doc + integration)
cargo test
```

### Integration tests

Integration tests live in `tests/integration/` and are wired up via `[[test]]` in `Cargo.toml`. They start a fresh `fbarresi/softplc:latest-linux` container per test using [testcontainers-rs](https://rust.testcontainers.org/) (`blocking` feature), wait for `"Application started."` in container stdout, then exercise connect/read/write/bit operations over the real S7 protocol.

**No manual `DOCKER_HOST` needed.** `ensure_docker_host()` in `common.rs` auto-detects the socket by probing these paths in order: `$HOME/.docker/run/docker.sock` (Docker Desktop), `/var/run/docker.sock` (Linux daemon), `/run/docker.sock`, then the Podman rootless socket. It only overrides `DOCKER_HOST` if the current value is absent, empty, or not a valid URL scheme — so an explicit `DOCKER_HOST=unix://...` still takes precedence.

**Test files:**
- `tests/integration/common.rs` — `start_softplc()` (incl. socket auto-detection), `provision_db()`, `connect_client()`
- `tests/integration/connection.rs` — connection lifecycle (connect, PDU negotiation, disconnect, reconnect)
- `tests/integration/read_write.rs` — `read_db`, `write_db`, `read_bit`, `write_bit`, auto-chunking
- `tests/integration/szl.rs` — SZL tests: `read_work_memory` passes against softplc; cycle-time and other SZL probes are `#[ignore]`'d (softplc returns errors for those IDs)
- `tests/integration/tis.rs` — 5 `#[ignore]`'d `VERIFY-ON-HARDWARE` tests for the experimental TIS TIMEMEAS path; require a real S7-300/400 CPU (`RUST7_HARDWARE_IP` env var) — SoftPLC silently drops TIS requests, confirmed via manual testing. See `docs/protocol/tis-timemeas.md`.

**Offline protocol tests** (`src/tests/tis.rs`, `src/tests/tshark_oracle.rs`, part of `cargo test --lib`): byte-exact codec tests for the TIS request/response encoding, plus an independent-dissector oracle that writes generated frames to a pcap and asks a real `tshark` binary to confirm they're well-formed S7comm framing. The oracle is gated behind `RUST7_TSHARK=1` *and* `tshark` on `PATH` — skips gracefully otherwise. Run with `RUST7_TSHARK=1 cargo test --lib tshark_oracle` after `brew install wireshark` (CLI-only formula) or `apt-get install tshark`.

Do not introduce new doc-test failures.

---

## Codebase Navigation

### File Map

| File | Lines | Role |
|---|---|---|
| `src/lib.rs` | 38 | Crate entry. `#![forbid(unsafe_code)]`, embeds README as crate docs, re-exports public surface. Add nothing here without adding to `src/client.rs` first. |
| `src/client.rs` | 1802 | All implementation: constants, macros, `S7Error`, `CpuFamily`, `S7Client`. The full protocol stack. |
| `src/tis.rs` | 164 | **Experimental**, crate-internal. TIS `TIMEMEAS` userdata subfunction — S7-300/400 cycle-time reads. Unverified against real hardware; see `docs/protocol/tis-timemeas.md`. |
| `src/diag_events.rs` | — | Diagnostic event ID lookup tables (557 entries) and `describe_event(u16) -> DiagEventInfo`. Derived from the Wireshark S7Comm dissector. |
| `Cargo.toml` | 22 | Zero `[dependencies]`. `[lib]` points to `src/lib.rs`. `[dev-dependencies]` has testcontainers. |
| `doc/Documentation.md` | 792 | Full API reference. Canonical source of truth for method semantics, parameters, and error codes. |
| `docs/protocol/tis-timemeas.md` | 323 | Original protocol write-up for TIS `TIMEMEAS`, citing the Wireshark dissector and Apache PLC4X `s7.mspec`. Documents what's verified vs. guessed, including two bugs the tshark oracle caught during development. |
| `examples/docker/main.rs` | 187 | Standalone binary (separate crate) demonstrating read/write/bit ops against SoftPLC. |
| `examples/docker/docker-compose.yml` | 10 | Launches SoftPLC on port 102 (S7) and 8080 (REST). |
| `examples/docker/Taskfile.yml` | 58 | Task runner. Key tasks: `build`, `run`, `docker-up`, `docker-down`, `dev`. |
| `examples/sdk/client.rs` | 87 | SDK reference: large read (462 bytes, auto-chunks), large write (1024 bytes), bit ops. |
| `examples/diagnostics/main.rs` | — | Standalone binary: `read_cpu_info`, raw `read_szl`, and `read_diagnostic_buffer` with formatted output. |
| `CHANGELOG.md` | 14 | Version history. Follow its format for new entries. |
| `tests/integration/main.rs` | 3 | Integration test binary root (declared via `[[test]]` in Cargo.toml). |
| `tests/integration/common.rs` | — | Shared helpers: `start_softplc()`, `provision_db()`, `connect_client()`. |
| `tests/integration/connection.rs` | — | Connection lifecycle integration tests (4 tests). |
| `tests/integration/read_write.rs` | — | Read/write/bit integration tests against SoftPLC (5 tests). |
| `tests/integration/tis.rs` | — | 5 `#[ignore]`'d `VERIFY-ON-HARDWARE` tests for TIS `TIMEMEAS`; require a real S7-300/400 CPU. |

### Key Files to Understand First

1. `src/client.rs` — everything is here. Read constants first, then `S7Error`, then `connect_tsap`, then `read_area`/`write_area`.
2. `doc/Documentation.md` — full method reference including connection types, error codes, and field descriptions.
3. `examples/docker/main.rs` — canonical usage example covering all major operations.

### Common Patterns

- **Connection helpers** (`connect_s71200_1500`, `connect_s7300`, `connect_rack_slot`) — all call `connect_tsap()` after computing TSAP values. Use `connect_rack_slot` as the template for any new helper.
- **Simplified I/O helpers** (`read_db`, `write_db`, `read_bit`, `write_bit`) — all delegate to `read_area`/`write_area` with fixed `area`/`wordlen`. Use this pattern for any new helper.
- **Macros** `hi_part!`, `lo_part!`, `make_u16!` — used throughout for byte splitting/assembly. Use them for any new protocol byte manipulation; do not inline the arithmetic.
- **Error recovery** — low-level errors require `client.disconnect()` before retry. High-level S7 errors (`S7NotFound`, `S7InvalidAddress`) do not.

### Finding Examples

- Adding a connection helper? Reference `connect_rack_slot` in `src/client.rs`.
- Adding a read/write helper? Reference `read_db` or `read_bit`.
- Adding a memory area constant? Reference the `S7_AREA_*` block and `check_area()`.
- Adding an error variant? Reference `S7Error` enum + `impl fmt::Display for S7Error`.

---

## Development Workflows

### Adding a New Memory Area

1. Add `pub const S7_AREA_X: u8 = 0xNN;` near the existing area constants in `src/client.rs`.
2. Add the constant to the `AREAS` array inside `check_area()`.
3. Re-export from `src/lib.rs`.

Note: Timer and Counter areas use different word-length encoding in some firmware versions. Verify against actual PLC captures before claiming compatibility.

### Adding a New S7Error Variant

1. Add the variant to the `S7Error` enum.
2. Add its `Display` arm in `impl fmt::Display for S7Error`.
3. Fix all exhaustive `match` arms (the compiler will list them).
4. Return it from the appropriate detection site.
5. Document it in the relevant method's `### Errors` doc section.

### Adding a New Connection Helper

All helpers call `connect_tsap()`. Compute `local_tsap` and `remote_tsap`, then delegate:

```rust
pub fn connect_s7400(&mut self, ip: &str, rack: u16, slot: u16) -> Result<(), S7Error> {
    self.connect_rack_slot(ip, rack, slot)
}
```

### Adding a Simplified Read/Write Helper

Wrap `read_area` or `write_area` with fixed `area` and/or `wordlen`:

```rust
pub fn read_merker(&mut self, start: u16, buffer: &mut [u8]) -> Result<(), S7Error> {
    self.read_area(S7_AREA_MK, 0, start, S7_WL_BYTE, buffer)
}
```

### SCADA Polling Loop

Canonical pattern — keep connection and I/O in the same loop, reconnect on failure:

```rust
loop {
    if !client.connected {
        if let Err(e) = client.connect_s71200_1500("192.168.0.100") {
            eprintln!("Connection failed: {e}");
        }
    }
    if client.connected {
        if let Err(e) = do_read_write(&mut client) {
            eprintln!("I/O error: {e} — disconnecting");
            client.disconnect();  // reconnect next iteration
        }
    }
    std::thread::sleep(Duration::from_millis(100));
}
```

`client.disconnect()` is idempotent and safe to call when already disconnected.

### Commit Style

Use [Conventional Commits](https://www.conventionalcommits.org/). Every commit must have a type prefix:

| Prefix | When to use |
|--------|-------------|
| `feat:` | new user-visible feature or API |
| `fix:` | bug fix |
| `test:` | adding or updating tests |
| `docs:` | CLAUDE.md, README, doc comments |
| `refactor:` | internal restructure with no behaviour change |
| `chore:` | dependency bumps, formatting, CI |
| `ai:` | agentic scaffolding, prompts, agent config |

**Subject line:** imperative mood, ≤72 chars. An em dash (`—`) may separate the what from the why in a single line when no body is needed (e.g. `docs: update CLAUDE.md — bare cargo test works`).

**Body (optional):** blank line after subject, then explain *why* and *how*. Use a short bullet list when the commit touches multiple distinct things.

**Co-authoring with Claude:** always append the trailer when Claude co-authors:
```
Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

**Granularity:** one logical change per commit. A Cargo.toml dep addition and the tests that need it can go together; source changes and doc updates should be separate commits.

---

### Keeping docs in sync

After any change, update the following files to the extent the change warrants:

| File | Update when… |
|---|---|
| `CLAUDE.md` | File map line counts change, new source files are added, test count changes, architecture or workflow changes |
| `README.md` | Public API surface changes (new features, new types/functions visible to users), Quick Start example becomes stale |
| `doc/Documentation.md` | Any public method, type, constant, or field is added, removed, or semantically changed — this is the canonical API reference |

Rule of thumb: a `feat:` commit almost always needs `doc/Documentation.md` touched. A `refactor:` or `fix:` commit needs it only if externally observable behaviour changes.

### PR Checklist

- [ ] `cargo build` passes
- [ ] `cargo clippy` is clean (0 warnings — keep it that way)
- [ ] `cargo test` introduces no new doc-test failures (4 pre-existing)
- [ ] `cargo doc --no-deps` generates cleanly
- [ ] Any new `pub` item is re-exported from `src/lib.rs`
- [ ] Any new `S7Error` variant is handled exhaustively in all match arms
- [ ] `CHANGELOG.md` entry drafted
- [ ] `CLAUDE.md` / `README.md` / `doc/Documentation.md` updated where applicable (see above)

---

## S7 Protocol Domain Knowledge

### Protocol Layer Stack

```
Application data
  └── S7Comm — application layer
        └── ISO 8073 / COTP — connection-oriented transport
              └── TPKT (RFC 1006) — TCP transport header
                    └── TCP/IP — port 102
```

**TPKT** (bytes 0–3): byte 0 = `0x03` (version), byte 1 = `0x00` (reserved), bytes 2–3 = total frame length (big-endian u16). Every frame starts with these 4 bytes.

**COTP** (bytes 4–6, data phase): `0x02 0xF0 0x80`. Byte 6 = `0x80` = EOT (last/only fragment). Fragmented COTP packets return `IsoFragmentedPacket`.

**S7Comm**: byte 7 = `0x32` (protocol ID), byte 8 = `0x01` (JOB request) or `0x03` (ACK_DATA response), PDU ref at 11–12, param length at 13–14, data length at 15–16.

### Three-Phase Connection Sequence

1. **TCP connect** to `{ip}:102` with the configured connect timeout.
2. **ISO CR (Connection Request)** — 22-byte COTP telegram with TSAP records. Server replies CC; byte 5 = `0xD0` = success.
3. **S7 PDU Negotiation** — 25-byte S7Comm requesting 480-byte PDU. Server replies 27 bytes; bytes 25–26 = negotiated PDU size stored in `self.pdu_length`.

### TSAP Addressing

```
remote_tsap = (conn_type << 8) + (rack * 0x20) + slot
```

| PLC | rack | slot | remote_tsap (CT_PG=0x01) |
|---|---|---|---|
| S7-1200 / S7-1500 | 0 | 0 | `0x0100` |
| S7-300 | 0 | 2 | `0x0102` |
| S7-400 / WinAC | hardware config | hardware config | computed |
| LOGO! / S7-200 | — | — | use `connect_tsap()` directly |

Local TSAP is always `0x0100` for the client.

### PDU Mechanics and Chunking

- Read payload capacity: `pdu_length - 18` → `self.max_rd_pdu_data` (see `src/client.rs:474`)
- Write payload capacity: `pdu_length - 28` → `self.max_wr_pdu_data` (see `src/client.rs:475`)
- Typical negotiated PDU = 480 bytes → max single-chunk read = 462 bytes, write = 452 bytes
- Auto-chunking in the `while offset < datasize` loops in `read_area`/`write_area`
- `client.chunks` = number of PDU round-trips in the last operation

### Memory Areas

| Constant | Value | PLC Notation | Description |
|---|---|---|---|
| `S7_AREA_PE` | `0x81` | I / E | Process Inputs — refreshed from physical inputs each OB1 cycle |
| `S7_AREA_PA` | `0x82` | Q / A | Process Outputs — written to physical outputs at end of OB1 |
| `S7_AREA_MK` | `0x83` | M | Merkers / Flags — internal memory, not mapped to I/O |
| `S7_AREA_DB` | `0x84` | DB | Data Blocks — most common for SCADA/MES integration |

### Bit Addressing Formula

The S7 address field is a 24-bit **bit** address. For byte access the library shifts: `byte_offset << 3`. For direct bit access:

```
start = byte_num * 8 + bit_idx
```

Example: `DB100.DBX 45.3` → `start = 45 * 8 + 3 = 363`. The helpers `read_bit`/`write_bit` compute this internally.

### PLC Families Quick Reference

| Family | Method | Rack | Slot | Notes |
|---|---|---|---|---|
| S7-1200 | `connect_s71200_1500()` | 0 | 0 | Disable "optimized block access" for byte-level reads |
| S7-1500 | `connect_s71200_1500()` | 0 | 0 | Optimized DB on by default — must disable in TIA Portal |
| S7-300 | `connect_s7300()` | 0 | 2 | Step 7 Classic or TIA |
| S7-400 | `connect_rack_slot()` | varies | varies | Rack/slot from hardware configuration |
| WinAC / PLCSIM | `connect_rack_slot()` | 0 | varies | PLCSIM exposes virtual rack 0 |
| LOGO! | `connect_tsap()` | — | — | TSAP from LOGO! Soft Comfort |
| S7-200 | `connect_tsap()` | — | — | Legacy; Micro/WIN specific TSAPs |
| Sinamics | `connect_rack_slot()` | varies | varies | CU320/CU340 via PROFINET |

---

## Constraints and Gotchas

**Optimized Data Blocks (S7-1500 default):**
Reads return `S7Error::S7InvalidAddress` (response code `0x05`). Fix in TIA Portal: DB properties → uncheck "Optimized block access." Hardware configuration issue, not a library bug.

**`client.connected` is a cached flag, not a network probe:**
Stays `true` after a cable disconnect. The error only surfaces on the next I/O call.

**Error recovery — disconnect before retry:**
`S7Error::Io`, `IsoInvalidHeader`, `IsoFragmentedPacket`, `IsoInvalidTelegram` → call `disconnect()` immediately. The ISO layer is stateful; a mid-stream error leaves both sides inconsistent. Reconnect from scratch.
`S7NotFound`, `S7InvalidAddress` → TCP connection is still valid; no disconnect needed.

**Port 102 on Linux requires elevated privileges:**
Ports below 1024 need `sudo`, `setcap cap_net_bind_service=+ep`, or NAT forwarding. Use `set_connection_port()` if redirecting.

**Bit read performance:**
Each `read_bit` = one full PDU round-trip (~1ms). To monitor multiple bits in nearby bytes, use `read_db` and extract with bitwise ops in Rust — reduces round-trips by a factor of 8+.

**Timeout defaults:**
Connect 3000ms, Read 1000ms, Write 500ms. Managed switches with spanning-tree may delay first connection up to 30 seconds. Use `set_timeout()` before connecting.

**Writing to `S7_AREA_PA`:**
Lands in the output process image, but OB1 re-copies its own output assignments to the image at the end of each scan. Write is silently overwritten on the next cycle unless the output is unassigned in the PLC program.

**Known clippy warnings:** none — `cargo clippy` is clean.

---

## Anti-Patterns

- Never `unwrap()` on `self.stream.as_mut()` outside the `self.connected` guard — that guard is the safety invariant.
- Never add `Clone` or `Copy` to `S7Client` — it wraps `TcpStream`.
- Never manually implement `Send`/`Sync` — thread safety is the caller's responsibility.
- Never propose connection pooling inside this library — single-connection by design.
- Never write to `S7_AREA_PE` — it is the read-only input image, overwritten by hardware each scan.
- Never change `READ_RES_LEN` or `WRITE_RES_LEN` without a Wireshark/S7 trace — wrong values cause silent data corruption.
- `impl Default for S7Client` must delegate to `S7Client::new()`.

---

## Troubleshooting

**`S7Error::S7InvalidAddress` on DB reads from S7-1500:**
The DB is in optimized mode. Go to TIA Portal → DB properties → uncheck "Optimized block access."

**Connection succeeds but reads return garbage:**
Check that `db_number`, `start`, and buffer size match what is actually configured in the PLC. The PLC does not validate that you are reading a meaningful address range.

**`S7Error::Io` during normal operation:**
Network interruption or PLC reset. Call `client.disconnect()` then reconnect. Do not retry on the same connection.

**`PduNegotiationFailed`:**
The PLC rejected the PDU negotiation. The PLC may be at capacity (too many concurrent S7 connections). Check how many active connections the PLC is configured to allow in TIA Portal under CPU properties.

**Connection to PLCSIM fails:**
PLCSIM exposes rack 0 but the slot number depends on the PLCSIM version. Try `connect_rack_slot("127.0.0.1", 0, 0)` and `connect_rack_slot("127.0.0.1", 0, 1)`.

**Port 102 permission denied on Linux:**
Run with `sudo`, or: `sudo setcap cap_net_bind_service=+ep ./target/debug/your_binary`.

**Slow `cargo build`:**
Use `cargo check` for correctness checks. The library has no proc macros or heavy dependencies — first build is fast relative to most Rust projects.

**Debugging packet-level issues:**
Use Wireshark with the `s7comm` dissector (built into recent Wireshark versions). Filter: `s7comm`. Compare captures against the byte arrays constructed in `connect_tsap`, `read_area`, and `write_area` in `src/client.rs`.

---

## Protocol Validator Sub-Agent

When making changes to byte-level telegram construction in `connect_tsap`, `read_area`, `write_area`, the shared userdata envelope (`build_userdata_request`, `read_userdata_response`), or `src/tis.rs`, or when adding protocol offset constants or memory area/word-length constants — invoke the protocol validator sub-agent at `.claude/agents/protocol-validator.md`. It reads the code and produces a field-by-field PASS/FAIL validation report. It makes no edits.
