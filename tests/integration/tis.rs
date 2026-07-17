// VERIFY-ON-HARDWARE tests for the experimental TIS TIMEMEAS path (crate::tis /
// S7Client::read_cycle_time on S7-300/400). See docs/protocol/tis-timemeas.md for the full
// protocol write-up and the "On-hardware validation procedure" section these tests follow.
//
// These CANNOT run against fbarresi/softplc (confirmed: a manual smoke test showed SoftPLC
// silently drops TIS requests — Err(Io(WouldBlock)) after the read timeout, no response at
// all) or against any Docker container — TIS TIMEMEAS is a genuinely undocumented protocol
// path (see the spec doc) that only a physical S7-300/400 CPU can confirm or refute. Every
// test below is #[ignore]'d by default and documents exactly one assumption this repository
// cannot verify offline. Set RUST7_HARDWARE_IP to a real CPU's address and run with
// `cargo test --test integration tis -- --ignored --nocapture` to exercise them.

use rust7::S7Client;
use std::env;

fn hardware_ip() -> String {
    env::var("RUST7_HARDWARE_IP")
        .expect("set RUST7_HARDWARE_IP to a real S7-300/400 CPU address to run this test")
}

// VERIFY-ON-HARDWARE (assumption 1 of 5): the exact TIS parameter register values that
// select "OB1 cycle time" for TIMEMEAS. src/tis.rs sends the minimal well-formed form with
// both registers zeroed (docs/protocol/tis-timemeas.md, "Rust7's request"). If a real CPU
// rejects this (non-success data return code, or a connection-level error), the zeroed
// registers are wrong and a real STEP 7 capture is needed to find the correct values.
#[test]
#[ignore = "VERIFY-ON-HARDWARE: requires a real S7-300/400 CPU (set RUST7_HARDWARE_IP)"]
fn timemeas_request_is_accepted_by_a_real_s7300_400_cpu() {
    let mut client = S7Client::new();
    client
        .connect_s7300(&hardware_ip())
        .expect("connect to the S7-300/400 CPU");

    match client.read_cycle_time() {
        Ok(info) => {
            println!("TIMEMEAS accepted, decoded (unverified layout): {info:?}");
        }
        Err(e) => {
            panic!(
                "TIMEMEAS request was rejected: {e}. This confirms the guessed request \
                 parameters in src/tis.rs are wrong — capture a real STEP 7 'Scan Cycle \
                 Time' session with Wireshark (see docs/protocol/tis-timemeas.md) to find \
                 the correct values."
            );
        }
    }
}

// VERIFY-ON-HARDWARE (assumption 2 of 5): the response field layout. src/tis.rs guesses the
// same 4x u32-big-endian-0.1ms-units layout as SZL 0x0194 on S7-1200/1500 (ob1_count, min,
// max, current). Compare the decoded values below against STEP 7's own "Scan Cycle Time"
// display for the same CPU at the same moment to confirm or refute this layout.
#[test]
#[ignore = "VERIFY-ON-HARDWARE: requires a real S7-300/400 CPU (set RUST7_HARDWARE_IP)"]
fn timemeas_response_layout_matches_step7_scan_cycle_time_display() {
    let mut client = S7Client::new();
    client
        .connect_s7300(&hardware_ip())
        .expect("connect to the S7-300/400 CPU");

    let info = client
        .read_cycle_time()
        .expect("TIMEMEAS request should succeed (see the other VERIFY-ON-HARDWARE test)");

    println!(
        "Decoded (guessed layout): ob1_count={}, min={}ms, max={}ms, current={}ms",
        info.ob1_count, info.min_ms, info.max_ms, info.current_ms
    );
    println!(
        "Compare against STEP 7 / TIA Portal -> Online & Diagnostics -> Module Information \
         -> Scan Cycle Time for the same CPU, observed at the same moment. If the numbers \
         don't correspond (e.g. current_ms isn't a plausible scan time, or min/max/current \
         look swapped or scaled), the guessed 16-byte layout in \
         tis::decode_cycle_time_guess() is wrong."
    );
}

// VERIFY-ON-HARDWARE (assumption 3 of 5): single-shot vs. job lifecycle. src/tis.rs sends
// TIMEMEAS as a single request/response (USERDATA_METHOD_SHORT, no continuation) — see
// docs/protocol/tis-timemeas.md "Single-shot vs. job" for the framing-level reasoning. If
// TIMEMEAS actually requires arming via ENABLEJOB/READJOB first, this call will most likely
// fail cleanly (a non-success return code) rather than silently misbehave — but confirming
// that requires comparing against a real STEP 7 capture to see whether it issues a job
// sequence (DISABLEJOB/ENABLEJOB/READJOB opcodes 0x0d-0x12) around its TIMEMEAS calls.
#[test]
#[ignore = "VERIFY-ON-HARDWARE: requires a real S7-300/400 CPU + a Wireshark capture of a STEP 7 session"]
fn timemeas_is_confirmed_single_shot_not_job_based() {
    panic!(
        "Not automatable — this assumption can only be confirmed by capturing a real STEP 7 \
         'Scan Cycle Time' session with Wireshark and checking whether STEP 7 issues \
         DISABLEJOB/ENABLEJOB/READJOB (subfunctions 0x0d-0x12) around its TIMEMEAS calls, or \
         a single direct TIMEMEAS request/response as src/tis.rs assumes. See \
         docs/protocol/tis-timemeas.md 'On-hardware validation procedure'."
    );
}

// VERIFY-ON-HARDWARE (assumption 4 of 5): connection type. S7Client defaults to CT_PG, which
// this test relies on implicitly (connect_s7300 doesn't override conn_type). If a real CPU
// requires CT_OP or CT_S7 specifically for TIS "Programmer commands" functions — or rejects
// them over a non-PG connection — that would surface here as a TIMEMEAS-specific rejection
// distinct from a general connection failure.
#[test]
#[ignore = "VERIFY-ON-HARDWARE: requires a real S7-300/400 CPU (set RUST7_HARDWARE_IP)"]
fn timemeas_works_over_default_pg_connection_type() {
    let mut client = S7Client::new();
    // CT_PG is the default; asserted explicitly here since this test exists specifically to
    // confirm that default is sufficient for TIS.
    assert_eq!(rust7::CT_PG, 0x0001);
    client
        .connect_s7300(&hardware_ip())
        .expect("connect to the S7-300/400 CPU over the default PG connection type");

    client
        .read_cycle_time()
        .expect("TIMEMEAS should succeed over a PG connection if CT_PG is sufficient");
}

// VERIFY-ON-HARDWARE (assumption 5 of 5): firmware variance across S7-300 and S7-400, and
// across firmware versions within each family. Re-run the tests above against at least one
// S7-300 CPU and one S7-400 CPU (connect via connect_rack_slot() for S7-400, since there is
// no dedicated connect_s7400() helper) with different firmware versions if available.
#[test]
#[ignore = "VERIFY-ON-HARDWARE: requires access to multiple real S7-300/400 CPUs with different firmware"]
fn timemeas_behaviour_is_consistent_across_firmware_versions() {
    panic!(
        "Not automatable in this environment — re-run the other tests in this file against \
         multiple physical CPUs (S7-300 and S7-400, different firmware versions) and compare \
         results. See docs/protocol/tis-timemeas.md 'Open risks' item 4."
    );
}
