use crate::S7Error;
use std::io;

fn display(e: &S7Error) -> String {
    format!("{}", e)
}

#[test]
fn io_error_non_empty() {
    let err = S7Error::Io(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
    assert!(!display(&err).is_empty());
}

#[test]
fn not_connected_message() {
    let msg = display(&S7Error::NotConnected);
    assert!(msg.to_lowercase().contains("connect"));
}

#[test]
fn tcp_connection_failed_non_empty() {
    assert!(!display(&S7Error::TcpConnectionFailed).is_empty());
}

#[test]
fn connection_closed_non_empty() {
    assert!(!display(&S7Error::ConnectionClosed).is_empty());
}

#[test]
fn iso_connection_failed_non_empty() {
    assert!(!display(&S7Error::IsoConnectionFailed).is_empty());
}

#[test]
fn iso_fragmented_packet_non_empty() {
    assert!(!display(&S7Error::IsoFragmentedPacket).is_empty());
}

#[test]
fn iso_invalid_header_non_empty() {
    assert!(!display(&S7Error::IsoInvalidHeader).is_empty());
}

#[test]
fn iso_invalid_telegram_non_empty() {
    assert!(!display(&S7Error::IsoInvalidTelegram).is_empty());
}

#[test]
fn pdu_negotiation_failed_non_empty() {
    assert!(!display(&S7Error::PduNegotiationFailed).is_empty());
}

#[test]
fn invalid_fun_parameter_non_empty() {
    assert!(!display(&S7Error::InvalidFunParameter).is_empty());
}

#[test]
fn s7_not_found_non_empty() {
    assert!(!display(&S7Error::S7NotFound).is_empty());
}

#[test]
fn s7_invalid_address_non_empty() {
    assert!(!display(&S7Error::S7InvalidAddress).is_empty());
}

#[test]
fn s7_unspecified_non_empty() {
    assert!(!display(&S7Error::S7Unspecified).is_empty());
}

#[test]
fn other_passthrough() {
    let msg = "custom error text";
    assert_eq!(display(&S7Error::Other(msg.to_string())), msg);
}

#[test]
fn unsupported_cpu_family_includes_family_and_feature() {
    let err = S7Error::UnsupportedCpuFamily {
        family: "S7-300/400",
        feature: "read_cycle_time()",
    };
    let msg = display(&err);
    assert!(msg.contains("S7-300/400"));
    assert!(msg.contains("read_cycle_time()"));
}
