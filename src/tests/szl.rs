// Private helpers are accessible to submodules via super::super (the client module).
use super::super::{
    build_szl_first_request, build_szl_next_request, parse_bcd_timestamp, SZL_REQ_LEN,
};
use crate::{
    CpuFamily, CpuInfo, CycleTimeInfo, S7Client, S7DateTime, S7Error, Szl, SzlHeader,
    WorkMemoryRecord,
};

// ── read_szl guard tests ─────────────────────────────────────────────────────

#[test]
fn read_work_memory_not_connected() {
    let mut c = S7Client::new();
    assert!(matches!(
        c.read_work_memory(),
        Err(S7Error::NotConnected)
    ));
}

#[test]
fn read_cycle_time_not_connected() {
    let mut c = S7Client::new();
    assert!(matches!(c.read_cycle_time(), Err(S7Error::NotConnected)));
}

// ── read_cycle_time CPU-family dispatch (exercised without a network connection) ──

#[test]
fn read_cycle_time_s7300_profile_returns_unsupported_cpu_family() {
    let mut c = S7Client::new();
    c.connect_profile = Some(CpuFamily::S7300);
    assert!(matches!(
        c.read_cycle_time(),
        Err(S7Error::UnsupportedCpuFamily { .. })
    ));
}

#[test]
fn read_cycle_time_s7400_profile_returns_unsupported_cpu_family() {
    let mut c = S7Client::new();
    c.connect_profile = Some(CpuFamily::S7400);
    assert!(matches!(
        c.read_cycle_time(),
        Err(S7Error::UnsupportedCpuFamily { .. })
    ));
}

#[test]
fn read_cycle_time_s71200_1500_profile_reaches_szl_path() {
    // Explicit 1200/1500 profile skips the family-dispatch error entirely and falls
    // through to the (unchanged) SZL read, which fails with NotConnected — not
    // UnsupportedCpuFamily — because no real connection was made.
    let mut c = S7Client::new();
    c.connect_profile = Some(CpuFamily::S71200_1500);
    assert!(matches!(c.read_cycle_time(), Err(S7Error::NotConnected)));
}

#[test]
fn read_cycle_time_other_profile_reaches_szl_path() {
    // `Other` (e.g. via connect_rack_slot with no CPU-info fallback available) preserves
    // the pre-existing SZL 0x0194 behaviour rather than rejecting the call outright.
    let mut c = S7Client::new();
    c.connect_profile = Some(CpuFamily::Other);
    assert!(matches!(c.read_cycle_time(), Err(S7Error::NotConnected)));
}

#[test]
fn read_szl_not_connected() {
    let mut c = S7Client::new();
    assert!(matches!(c.read_szl(0x0011, 0), Err(S7Error::NotConnected)));
}

#[test]
fn read_diagnostic_buffer_not_connected() {
    let mut c = S7Client::new();
    assert!(matches!(
        c.read_diagnostic_buffer(),
        Err(S7Error::NotConnected)
    ));
}

#[test]
fn read_cpu_info_not_connected() {
    let mut c = S7Client::new();
    assert!(matches!(c.read_cpu_info(), Err(S7Error::NotConnected)));
}

// ── SZL-ID constant values ───────────────────────────────────────────────────

#[test]
fn szl_id_constants_have_expected_values() {
    assert_eq!(crate::S7_SZL_CPU_ID, 0x0011);
    assert_eq!(crate::S7_SZL_WORK_MEMORY, 0x0013);
    assert_eq!(crate::S7_SZL_CPU_INFO, 0x001C);
    assert_eq!(crate::S7_SZL_DIAG_BUFFER, 0x00A0);
    assert_eq!(crate::S7_SZL_CYCLE_TIME, 0x0194);
    assert_eq!(crate::S7_SZL_CPU_STATUS, 0x0424);
}

// ── Error Display ─────────────────────────────────────────────────────────────

#[test]
fn szl_read_failed_display_non_empty() {
    let msg = format!("{}", S7Error::SzlReadFailed);
    assert!(!msg.is_empty());
    assert!(msg.to_lowercase().contains("szl"));
}

// ── Request telegram encoding ─────────────────────────────────────────────────

#[test]
fn first_request_is_33_bytes() {
    let req = build_szl_first_request(0x0011, 0x0000);
    assert_eq!(req.len(), SZL_REQ_LEN);
    assert_eq!(SZL_REQ_LEN, 33);
}

#[test]
fn first_request_tpkt_length_field() {
    let req = build_szl_first_request(0x0011, 0x0000);
    let total = ((req[2] as u16) << 8) | req[3] as u16;
    assert_eq!(total, 33, "TPKT length field must equal telegram size");
}

#[test]
fn first_request_tpkt_iso_header_constants() {
    let req = build_szl_first_request(0x0011, 0x0000);
    assert_eq!(req[0], 0x03, "TPKT version");
    assert_eq!(req[4], 0x02, "COTP length");
    assert_eq!(req[5], 0xF0, "COTP PDU type");
    assert_eq!(req[6], 0x80, "EOT");
}

#[test]
fn first_request_s7_userdata_header() {
    let req = build_szl_first_request(0x0011, 0x0000);
    assert_eq!(req[7], 0x32, "S7 protocol ID");
    assert_eq!(req[8], 0x07, "ROSCTR = Userdata");
    let plen = ((req[13] as u16) << 8) | req[14] as u16;
    assert_eq!(plen, 8, "parameter length");
    let dlen = ((req[15] as u16) << 8) | req[16] as u16;
    assert_eq!(dlen, 8, "data length");
}

#[test]
fn first_request_param_block_fields() {
    let req = build_szl_first_request(0x0011, 0x0000);
    // Param header magic at 17-19
    assert_eq!(&req[17..20], &[0x00, 0x01, 0x12]);
    assert_eq!(req[21], 0x11, "method = request");
    assert_eq!(req[22], 0x44, "type=4(req)|group=4(CPU)");
    assert_eq!(req[23], 0x01, "subfunction = ReadSZL");
    assert_eq!(req[24], 0x00, "sequence number = 0 (first)");
}

#[test]
fn first_request_szl_id_big_endian() {
    let req = build_szl_first_request(0x00A0, 0x0000);
    assert_eq!(req[29], 0x00, "SZL-ID hi");
    assert_eq!(req[30], 0xA0, "SZL-ID lo");
}

#[test]
fn first_request_szl_index_big_endian() {
    let req = build_szl_first_request(0x0011, 0x0102);
    assert_eq!(req[31], 0x01, "SZL-INDEX hi");
    assert_eq!(req[32], 0x02, "SZL-INDEX lo");
}

#[test]
fn next_request_is_33_bytes() {
    assert_eq!(build_szl_next_request(0x05).len(), SZL_REQ_LEN);
}

#[test]
fn next_request_tpkt_length_field() {
    let req = build_szl_next_request(0x05);
    let total = ((req[2] as u16) << 8) | req[3] as u16;
    assert_eq!(total, 33);
}

#[test]
fn next_request_rosctr_userdata() {
    assert_eq!(build_szl_next_request(0x05)[8], 0x07);
}

#[test]
fn next_request_seq_num_at_offset_24() {
    let req = build_szl_next_request(0xAB);
    assert_eq!(req[24], 0xAB);
}

#[test]
fn next_request_param_length_is_12() {
    let req = build_szl_next_request(0x01);
    let plen = ((req[13] as u16) << 8) | req[14] as u16;
    assert_eq!(plen, 12);
}

// ── BCD timestamp decoding ───────────────────────────────────────────────────

#[test]
fn bcd_timestamp_decodes_correctly() {
    // 2024-03-15 14:30:05.225 weekday=4
    // byte6 = 0x25 → bcd=25 (tens+units of ms)
    // byte7 high nibble = 2 → hundreds of ms; low nibble = 4 (weekday)
    // millisecond = 2*100 + 25 = 225
    let data: [u8; 8] = [0x24, 0x03, 0x15, 0x14, 0x30, 0x05, 0x25, 0x24];
    let dt = parse_bcd_timestamp(&data).expect("valid BCD");
    assert_eq!(dt.year, 2024);
    assert_eq!(dt.month, 3);
    assert_eq!(dt.day, 15);
    assert_eq!(dt.hour, 14);
    assert_eq!(dt.minute, 30);
    assert_eq!(dt.second, 5);
    assert_eq!(dt.millisecond, 225);
    assert_eq!(dt.weekday, 4);
}

#[test]
fn bcd_year_90_maps_to_1990() {
    let mut d = [0u8; 8];
    d[0] = 0x90;
    assert_eq!(parse_bcd_timestamp(&d).unwrap().year, 1990);
}

#[test]
fn bcd_year_89_maps_to_2089() {
    let mut d = [0u8; 8];
    d[0] = 0x89;
    assert_eq!(parse_bcd_timestamp(&d).unwrap().year, 2089);
}

#[test]
fn bcd_year_00_maps_to_2000() {
    let mut d = [0u8; 8];
    d[0] = 0x00;
    assert_eq!(parse_bcd_timestamp(&d).unwrap().year, 2000);
}

#[test]
fn bcd_year_99_maps_to_1999() {
    let mut d = [0u8; 8];
    d[0] = 0x99;
    assert_eq!(parse_bcd_timestamp(&d).unwrap().year, 1999);
}

#[test]
fn bcd_invalid_nibble_returns_none() {
    let data: [u8; 8] = [0xAB, 0, 0, 0, 0, 0, 0, 0]; // 0xAB not valid BCD
    assert!(parse_bcd_timestamp(&data).is_none());
}

#[test]
fn bcd_invalid_ms_hundreds_nibble_returns_none() {
    // byte7 high nibble = 0xA (>9)
    let data: [u8; 8] = [0x24, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0xA4];
    assert!(parse_bcd_timestamp(&data).is_none());
}

#[test]
fn bcd_all_zero_is_valid() {
    let dt = parse_bcd_timestamp(&[0u8; 8]).expect("all-zero BCD is valid");
    assert_eq!(dt.year, 2000);
    assert_eq!(dt.millisecond, 0);
    assert_eq!(dt.weekday, 0);
}

// ── Diagnostic entry decoder (exercised without a network connection) ─────────

fn make_raw_diag_entry(event_id: u16, year_bcd: u8) -> [u8; 20] {
    let mut e = [0u8; 20];
    e[0] = (event_id >> 8) as u8;
    e[1] = (event_id & 0xFF) as u8;
    e[2] = year_bcd; // timestamp byte 0 = year BCD
    e
}

fn decode_diag_entries(szl: &Szl) -> Vec<(u16, Option<S7DateTime>, [u8; 10])> {
    const ENTRY_LEN: usize = 20;
    let mut out = Vec::new();
    let mut off = 0;
    while off + ENTRY_LEN <= szl.data.len() {
        let rec = &szl.data[off..off + ENTRY_LEN];
        let event_id = ((rec[0] as u16) << 8) | rec[1] as u16;
        let mut ts = [0u8; 8];
        ts.copy_from_slice(&rec[2..10]);
        let timestamp = parse_bcd_timestamp(&ts);
        let mut info = [0u8; 10];
        info.copy_from_slice(&rec[10..20]);
        out.push((event_id, timestamp, info));
        off += ENTRY_LEN;
    }
    out
}

#[test]
fn diag_decode_two_entries() {
    let mut data = Vec::new();
    data.extend_from_slice(&make_raw_diag_entry(0x1234, 0x24)); // year 2024
    data.extend_from_slice(&make_raw_diag_entry(0xABCD, 0x99)); // year 1999
    let szl = Szl {
        header: SzlHeader {
            length_dr: 20,
            n_dr: 2,
        },
        data,
    };
    let entries = decode_diag_entries(&szl);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, 0x1234);
    assert_eq!(entries[0].1.unwrap().year, 2024);
    assert_eq!(entries[1].0, 0xABCD);
    assert_eq!(entries[1].1.unwrap().year, 1999);
}

#[test]
fn diag_decode_truncated_record_is_skipped() {
    // 19 bytes — one short of a complete 20-byte entry
    let szl = Szl {
        header: SzlHeader {
            length_dr: 20,
            n_dr: 1,
        },
        data: vec![0u8; 19],
    };
    assert!(decode_diag_entries(&szl).is_empty());
}

#[test]
fn diag_decode_empty_data_returns_empty_vec() {
    let szl = Szl {
        header: SzlHeader {
            length_dr: 20,
            n_dr: 0,
        },
        data: vec![],
    };
    assert!(decode_diag_entries(&szl).is_empty());
}

// ── CpuInfo record decoder (exercised without a network connection) ────────────

fn make_cpu_record(idx: u16, text: &str, rec_len: usize) -> Vec<u8> {
    let mut rec = vec![0u8; rec_len];
    rec[0] = (idx >> 8) as u8;
    rec[1] = (idx & 0xFF) as u8;
    let b = text.as_bytes();
    let n = b.len().min(rec_len - 2);
    rec[2..2 + n].copy_from_slice(&b[..n]);
    rec
}

fn decode_cpu_info(szl: &Szl) -> CpuInfo {
    let rec_len = szl.header.length_dr as usize;
    let mut info = CpuInfo::default();
    if rec_len < 4 {
        return info;
    }
    let mut off = 0;
    while off + rec_len <= szl.data.len() {
        let rec = &szl.data[off..off + rec_len];
        let idx = ((rec[0] as u16) << 8) | rec[1] as u16;
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
    info
}

#[test]
fn cpu_info_all_known_indices_parsed() {
    const RL: usize = 36;
    let mut data = Vec::new();
    for (idx, text) in [
        (1u16, "CPU 1214C"),
        (2, "My PLC"),
        (3, "Line1"),
        (6, "Siemens"),
        (7, "S1234"),
    ] {
        data.extend(make_cpu_record(idx, text, RL));
    }
    let szl = Szl {
        header: SzlHeader {
            length_dr: RL as u16,
            n_dr: 5,
        },
        data,
    };
    let info = decode_cpu_info(&szl);
    assert_eq!(info.module_type_name, "CPU 1214C");
    assert_eq!(info.module_name, "My PLC");
    assert_eq!(info.as_name, "Line1");
    assert_eq!(info.copyright, "Siemens");
    assert_eq!(info.serial_number, "S1234");
}

#[test]
fn cpu_info_unknown_index_ignored() {
    const RL: usize = 34;
    let mut data = Vec::new();
    data.extend(make_cpu_record(99, "ignored", RL));
    data.extend(make_cpu_record(1, "Known", RL));
    let szl = Szl {
        header: SzlHeader {
            length_dr: RL as u16,
            n_dr: 2,
        },
        data,
    };
    let info = decode_cpu_info(&szl);
    assert_eq!(info.module_type_name, "Known");
    assert!(info.serial_number.is_empty());
}

// ── WorkMemoryRecord decoder (exercised without a network connection) ──────────

fn decode_work_memory(szl: &Szl) -> Vec<WorkMemoryRecord> {
    let rec_len = szl.header.length_dr as usize;
    let mut records = Vec::new();
    let mut off = 0;
    while off + rec_len <= szl.data.len() {
        let rec = &szl.data[off..off + rec_len];
        if rec.len() >= 12 {
            records.push(WorkMemoryRecord {
                index:       ((rec[0] as u16) << 8) | rec[1] as u16,
                area_type:   ((rec[2] as u16) << 8) | rec[3] as u16,
                total_bytes: u32::from_be_bytes([rec[4], rec[5], rec[6], rec[7]]),
                used_bytes:  u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]),
            });
        }
        off += rec_len;
    }
    records
}

fn make_work_memory_record(index: u16, area_type: u16, total: u32, used: u32, rec_len: usize) -> Vec<u8> {
    let mut rec = vec![0u8; rec_len];
    rec[0] = (index >> 8) as u8;
    rec[1] = (index & 0xFF) as u8;
    rec[2] = (area_type >> 8) as u8;
    rec[3] = (area_type & 0xFF) as u8;
    rec[4..8].copy_from_slice(&total.to_be_bytes());
    rec[8..12].copy_from_slice(&used.to_be_bytes());
    rec
}

#[test]
fn work_memory_parse_single_record() {
    const RL: usize = 34;
    let data = make_work_memory_record(0x0001, 0x0003, 0x00080000, 0x00012345, RL);
    let szl = Szl {
        header: SzlHeader { length_dr: RL as u16, n_dr: 1 },
        data,
    };
    let records = decode_work_memory(&szl);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].index, 0x0001);
    assert_eq!(records[0].area_type, 0x0003);
    assert_eq!(records[0].total_bytes, 0x00080000);
    assert_eq!(records[0].used_bytes, 0x00012345);
}

#[test]
fn work_memory_parse_multiple_records() {
    const RL: usize = 34;
    let mut data = Vec::new();
    data.extend(make_work_memory_record(0x0001, 0x0001, 100_000, 40_000, RL));
    data.extend(make_work_memory_record(0x0002, 0x0002, 200_000, 80_000, RL));
    data.extend(make_work_memory_record(0x0003, 0x0004,  50_000, 10_000, RL));
    let szl = Szl {
        header: SzlHeader { length_dr: RL as u16, n_dr: 3 },
        data,
    };
    let records = decode_work_memory(&szl);
    assert_eq!(records.len(), 3);
    assert_eq!(records[1].total_bytes, 200_000);
    assert_eq!(records[2].used_bytes, 10_000);
}

#[test]
fn work_memory_empty_data_returns_empty_vec() {
    let szl = Szl {
        header: SzlHeader { length_dr: 34, n_dr: 0 },
        data: vec![],
    };
    assert!(decode_work_memory(&szl).is_empty());
}

#[test]
fn work_memory_short_record_skipped() {
    // rec_len = 34, but only 11 bytes supplied — guard `rec.len() >= 12` skips it
    let szl = Szl {
        header: SzlHeader { length_dr: 34, n_dr: 1 },
        data: vec![0u8; 11],
    };
    assert!(decode_work_memory(&szl).is_empty());
}

// ── CycleTimeInfo decoder (exercised without a network connection) ─────────────

fn decode_cycle_time(data: &[u8]) -> Result<CycleTimeInfo, S7Error> {
    if data.len() < 18 {
        return Err(S7Error::IsoInvalidTelegram);
    }
    Ok(CycleTimeInfo {
        ob1_count:  u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
        min_ms:     u32::from_be_bytes([data[6],  data[7],  data[8],  data[9]])  as f64 / 10.0,
        max_ms:     u32::from_be_bytes([data[10], data[11], data[12], data[13]]) as f64 / 10.0,
        current_ms: u32::from_be_bytes([data[14], data[15], data[16], data[17]]) as f64 / 10.0,
    })
}

#[test]
fn cycle_time_parse_known_values() {
    // index=0x0001, count=42, min=500 (50.0ms), max=1200 (120.0ms), current=600 (60.0ms)
    let mut data = vec![0u8; 28];
    data[0] = 0x00; data[1] = 0x01;             // index
    data[2..6].copy_from_slice(&42u32.to_be_bytes());   // ob1_count
    data[6..10].copy_from_slice(&500u32.to_be_bytes()); // min (0.1ms units)
    data[10..14].copy_from_slice(&1200u32.to_be_bytes()); // max
    data[14..18].copy_from_slice(&600u32.to_be_bytes());  // current
    let ct = decode_cycle_time(&data).expect("valid cycle time");
    assert_eq!(ct.ob1_count, 42);
    assert!((ct.min_ms - 50.0).abs() < f64::EPSILON);
    assert!((ct.max_ms - 120.0).abs() < f64::EPSILON);
    assert!((ct.current_ms - 60.0).abs() < f64::EPSILON);
}

#[test]
fn cycle_time_zero_values_are_valid() {
    let data = vec![0u8; 28];
    let ct = decode_cycle_time(&data).expect("all-zero is valid");
    assert_eq!(ct.ob1_count, 0);
    assert_eq!(ct.min_ms, 0.0);
}

#[test]
fn cycle_time_data_too_short_returns_error() {
    assert!(matches!(
        decode_cycle_time(&[0u8; 17]),
        Err(S7Error::IsoInvalidTelegram)
    ));
}

#[test]
fn cpu_info_null_padding_trimmed() {
    const RL: usize = 36;
    // "CPU\0\0\0..." — null padding after text
    let text = "CPU";
    let data = make_cpu_record(1, text, RL);
    let szl = Szl {
        header: SzlHeader {
            length_dr: RL as u16,
            n_dr: 1,
        },
        data,
    };
    let info = decode_cpu_info(&szl);
    assert_eq!(info.module_type_name, "CPU");
}
