<!-- markdownlint-disable blanks-around-headings blanks-around-lists no-duplicate-heading -->

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->
## [TBD] - Release date

### Added
### Changed
### Fixed

## [0.4.2] - 2024-11-28

### Changed

- Relaxed visibility of some intermediate-structs on from 
parsing internals to public, see https://github.com/MarcusGrass/http-range-header/issues/9

## [0.4.1] - 2024-05-03

### Fixed

- Panic at validation step if file-size doesn't make sense with the range, 
should have been an Error, thanks @cholcombe973

## [0.4.0] - 2023-07-21
### Added
- Bench error performance

### Changed
- Msrv set to 1.60.0 to match [tower-http](https://github.com/tower-rs/tower-http)
- Use higher `Rust` version features to improve performance
- Remove feature `with_error_cause`
- Convert Error into an enum

### Fixed

## [0.3.1] - 2023-07-21

### Fixed
- Now accepts ranges that are out of bounds, but truncates them down to an in-range 
value, according to the spec, thanks @jfaust!
- Clean up with clippy pedantic, update docs, format, etc. Resulted in a bench improvement of almost
5%.

## [0.3.0] - 2021-11-25

### Changed

- Only expose a single error-type to make usage more ergonomic

## [0.2.1] - 2021-11-25

### Added

- Make some optimizations

## [0.2.0] - 2021-11-25

### Changed

- Rename to http-range-header

## [0.1.0] - 2021-11-25

### Added

- Released first version under parse_range_headers
