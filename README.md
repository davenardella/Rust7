# Rust7 - Native Rust S7Client

[![Rust CI Badge](https://github.com/davenardella/Rust7/actions/workflows/ci.yml/badge.svg)](https://github.com/davenardella/Rust7/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/rust7.svg)](https://crates.io/crates/rust7) ![crates.io total downloads](https://img.shields.io/crates/d/rust7)

Pragmatic native Rust S7 client (Snap7‑style) for Siemens PLCs. 

---

## Features
- Pure Rust, no unsafe code.
- Low latency: ≈ 1ms/PDU.
- Small footprint.
- Strict control of incoming headers.
- Automatic telegram splitting for large reads/writes.
- Connection helpers methods for S71200/1500 and S7300.
---

## Quick start
```rust
use rust7::client::{S7Client};

fn main() {
    let mut client = S7Client::new();
    let db_number: u16 = 100; // Must exist into the PLC

    // Connection
    match client.connect_s71200_1500("192.168.0.100") {
        Ok(_) => { println!("Connected to PLC") },
        Err(e) => {
            eprintln!("Connection failed: {}", e);
            return;
        }
    }
    // Reads 64 byte from DB100
    println!("");
    println!("Attempt to read 64 byte from DB100");
    let mut read_buffer = vec![0u8; 64];
    match client.read_db(db_number, 0, &mut read_buffer) {
        Ok(_) => {          
            println!("Success!");
            println!("Job time (ms) : {:.3}", client.last_time);
        },
        Err(e) => eprintln!("Read failed: {}", e),
    }

    client.disconnect();
}
```

## Documentation
The detailed documentation is <a href="doc/Documentation.md" target="_blank">here</a>.

---

## License
Copyright © 2025 Davide Nardella

Distribuited under <a href="LICENSE" target="_blank">MIT License</a>. 
