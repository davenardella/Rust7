use crate::S7Client;

#[test]
fn starts_disconnected() {
    let client = S7Client::new();
    assert!(!client.connected);
}

#[test]
fn pdu_length_zero() {
    let client = S7Client::new();
    assert_eq!(client.pdu_length, 0);
}

#[test]
fn chunks_zero() {
    let client = S7Client::new();
    assert_eq!(client.chunks, 0);
}

#[test]
fn last_time_zero() {
    let client = S7Client::new();
    assert_eq!(client.last_time, 0.0);
}

#[test]
fn connect_profile_starts_unset() {
    let client = S7Client::new();
    assert_eq!(client.connect_profile, None);
}
