# Changelog

---

## [Unreleased]

### Added
- `read_work_memory()` — reads work memory area sizes from SZL `0x0013` (`WorkMemoryRecord`)
- `read_cycle_time()` — reads OB1 scan cycle time statistics from SZL `0x0194` (`CycleTimeInfo`)
- `S7_SZL_WORK_MEMORY` (`0x0013`) and `S7_SZL_CYCLE_TIME` (`0x0194`) constants
- Integration test for `read_work_memory` (passes against `fbarresi/softplc`)
- `CpuFamily` enum and `S7Client::connect_profile` field, recorded by `connect_s71200_1500()` /
  `connect_s7300()` / `connect_rack_slot()` and used to dispatch family-specific behaviour
- `S7Error::UnsupportedCpuFamily` — reserved for a future capability check; not currently
  returned by any method (see below)
- **Experimental:** `read_cycle_time()` on S7-300/400 now attempts a cycle-time read via the
  TIS `TIMEMEAS` userdata subfunction instead of failing outright. This path is unverified
  against real hardware — see `docs/protocol/tis-timemeas.md` for the full protocol write-up,
  citations, and the recommended DB-publish production alternative. A PLC-level rejection
  surfaces as a typed error; a response the PLC accepts may still decode into a
  `CycleTimeInfo` that isn't actually meaningful, pending hardware confirmation.
- `src/tis.rs` (crate-internal): TIS TIMEMEAS request builder and response parser
- Generic ROSCTR-0x07 Userdata request/response envelope (`build_userdata_request`,
  `read_userdata_response` in `src/client.rs`), extracted from the SZL-specific builders and
  shared by both the SZL and TIS paths

### Fixed
- `read_cycle_time()` on S7-300/400 no longer returns the misleading
  `S7Error::IsoInvalidTelegram` (SZL `0x0194` is S7-1200/1500-only); it now attempts the
  experimental TIS path described above

---

## [0.1.2] - 2025-08-15

### Added
- Added parameter check
- Added `InvalidFunParam`

### Modified
- Excluded /target folder from deploy

## [0.1.1] - 2025-08-14
- Initial release
