#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod client;
mod diag_events;
mod tis;

pub use client::{
    CpuFamily,
    CpuInfo,
    CycleTimeInfo,
    DiagnosticEntry,
    S7Client,
    S7DateTime,
    S7Error,
    WorkMemoryRecord,
    // SZL types
    Szl,
    SzlHeader,
    CT_OP,
    CT_PG,
    CT_S7,
    S7_AREA_DB,
    S7_AREA_MK,
    S7_AREA_PA,
    S7_AREA_PE,
    // SZL-ID constants
    S7_SZL_CPU_ID,
    S7_SZL_CPU_INFO,
    S7_SZL_CPU_STATUS,
    S7_SZL_CYCLE_TIME,
    S7_SZL_DIAG_BUFFER,
    S7_SZL_WORK_MEMORY,
    S7_WL_BIT,
    S7_WL_BYTE,
};

pub use diag_events::{describe_event, DiagEventInfo};
