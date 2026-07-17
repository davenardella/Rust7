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
- `S7Error::UnsupportedCpuFamily` — returned when a feature is requested via a mechanism the
  connected CPU family doesn't support

### Fixed
- `read_cycle_time()` on S7-300/400 now returns `S7Error::UnsupportedCpuFamily` (naming SZL
  `0x0194` as S7-1200/1500-only and suggesting a DB-publish) instead of the misleading
  `S7Error::IsoInvalidTelegram`

---

## [0.1.2] - 2025-08-15

### Added
- Added parameter check
- Added `InvalidFunParam`

### Modified
- Excluded /target folder from deploy

## [0.1.1] - 2025-08-14
- Initial release
