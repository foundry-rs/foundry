# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

# Unreleased

### Breaking changes

### Added

### Removed

### Changed

### Fixed

# [0.5.0] - 2024-10-28

### Changed

- Made `Report::build` accept a proper span, avoiding much type annotation trouble

# [0.4.1] - 2024-04-25

### Added

- Support for byte spans

- The ability to fetch the underlying `&str` of a `Source` using `source.as_ref()`

### Changed

- Upgraded `yansi` to `1.0`

# [0.4.0] - 2024-01-01

### Breaking changes

- Added missing `S: Span` bound for `Label::new` constructor.

- Previously labels with backwards spans could be constructed and
  only resulted in a panic when writing (or printing) the report.
  Now `Label::new` panics immediately when passed a backwards span.

### Added

- Support for alternative string-like types in `Source`

### Changed

- Memory & performance improvements

### Fixed

- Panic when provided with an empty input

- Invalid unicode characters for certain arrows

# [0.3.0] - 2023-06-07

### Changed

- Upgraded concolor to `0.1`.
