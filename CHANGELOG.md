# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.3] - 2026-05-27

### Added

- `[tip_prep.signal_stability]` config section exposing the signal-read stability
  gates at runtime: `max_std_dev_hz`, `max_slope_hz_per_s`,
  `data_collection_duration_ms`, `read_timeout_secs`, `read_retry_count`. These
  were previously compile-time only. Honored by both the CLI and the GUI (the GUI
  carries them through from the loaded config file).

### Changed

- Loosened the default noise gate `max_std_dev` from 0.3 → 1.5 Hz, which was too
  tight for typical tips (they fluctuate but hold a stable mean). Tune per setup
  via the new config section.

## [0.2.2] - 2026-05-27

### Fixed

- Frequency-shift drift is now measured in Hz/s instead of Hz-per-sample, so the
  stability check no longer varies with how many TCP frames were buffered
  (oversampling). A genuinely stable tip is now judged consistently.
- When a stable signal can't be confirmed, the reading falls back to the *mean* of
  the raw buffer instead of its *minimum*, removing a systematic negative bias in
  the reported frequency shift.
- Reading scan properties at the start of a stability sweep no longer fails with an
  `UnexpectedEof` IO error on older Nanonis firmware (via nanonis-rs 0.4.0's
  version-tolerant `Scan.PropsGet`).

### Changed

- Tightened the signal-stability gates to a realistic tip scale: noise threshold
  `max_std_dev` 1.0 → 0.3 Hz and drift threshold `max_slope` 2.0 → 0.5 Hz/s
  (≈0.25 Hz over the 500 ms collection window).
- Updated the nanonis-rs backend to 0.4.0.

## [0.2.1] - 2026-05-21

### Added

- `CITATION.cff` and Zenodo DOI badges for software citation metadata.
- crates.io publishing workflow that runs on version tags.

### Fixed

- Malformed author email in `Cargo.toml`.

## [0.2.0] - 2026-03-09

- First tagged release.
