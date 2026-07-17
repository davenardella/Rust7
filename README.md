# Rust7 - Native Rust S7Client

Pragmatic native Rust S7 client (Snap7‑style) for Siemens PLCs. 

---

## Features
- Pure Rust, zero unsafe code, zero external dependencies.
- Low latency: ≈ 1 ms/PDU.
- Automatic telegram splitting for reads/writes larger than the negotiated PDU size.
- Connection helpers for S7-1200/1500, S7-300, and rack/slot-based PLCs.
- SZL (System Status List) reads: raw `read_szl`, structured `read_cpu_info`, and `read_diagnostic_buffer`.
- Diagnostic event ID lookup: `describe_event` maps any `event_id` to a human-readable class and name.
- CPU-family-aware `read_cycle_time`: works on S7-1200/1500 via SZL; returns a clear `S7Error::UnsupportedCpuFamily` on S7-300/400, which expose no cycle-time SZL.

---

## Quick start
```rust
use rust7::{S7Client, S7Error};

fn main() -> Result<(), S7Error> {
    let mut client = S7Client::new();

    client.connect_s71200_1500("192.168.0.100")?;
    println!("Connected — PDU {} bytes", client.pdu_length);
    // Connected — PDU 480 bytes

    // Read 64 bytes from DB100, starting at byte 0
    let mut buf = vec![0u8; 64];
    client.read_db(100, 0, &mut buf)?;
    println!("Read {} bytes in {:.3} ms", buf.len(), client.last_time);
    // Read 64 bytes in 1.234 ms

    client.disconnect();
    Ok(())
}
```

## Documentation
The detailed documentation is <a href="doc/Documentation.md" target="_blank">here</a>.

## Examples
For practical examples see the documentation <a href="examples/README.md" target="_blank">here</a>.

## Debugging (Zed)
A sample Zed debug configuration is provided at [`.zed/debug.json.sample`](.zed/debug.json.sample).
Copy it to `.zed/debug.json` (gitignored) and adjust the PLC IP or adapter as needed.
Requires the [CodeLLDB](https://github.com/vadimcn/codelldb) Zed extension.

---

## License
Copyright © 2025 Davide Nardella

Distribuited under <a href="LICENSE" target="_blank">MIT License</a>.
