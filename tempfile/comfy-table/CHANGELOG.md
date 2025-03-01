# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [7.1.4] - 2025-02-07

### Fix

- Handle UTF-8 graphemes when truncating cells. [#167](https://github.com/Nukesor/comfy-table/pull/167)
- Respect UTF-8 zero-width joiner and variation selection characters when splitting words. [#168](https://github.com/Nukesor/comfy-table/pull/168) by [tisonkun](https://github.com/tisonkun)

### Change

- Remove strum dependency. [#169](https://github.com/Nukesor/comfy-table/pull/169) by [tisonkun](https://github.com/tisonkun)
- Introduce the `unicode-segmentation` library in the scope of #167 and #168.
- The new changes for correct UTF-8 handling have a performance hit of up to ~67%.
  However, this will most likely unnoticable for most people.
  The benchmark table with 10 columns and 500 rows slowed down from 15ms to 25ms.
  For "normal" tables, the performance hit is negligible.

## Chore

- Bump ansi-str

## [7.1.3] - 2024-11-24

### Fix

- Bump `crossterm` to `0.28` on Windows as well

## [7.1.2] - 2024-11-24

### Chore

- Bump `crossterm` to `0.28`
- Bump `unicode-width` to `0.2`

## [7.1.1] - 2024-04-05

## Fix

- Fix string width calculation with ANSI escape sequences by switching to the `ansi-str` crate.
  The previous implementation didn't respect OSC 8 hyperlink style ANSI sequences.
  Implemented by [dsully](https://github.com/dsully) in [#137](https://github.com/Nukesor/comfy-table/pull/137).
- `custom_styling` feature now requires `tty` feature as it should.

## [7.1.0] - 2023-10-21

### Added

- Add helper methods `(col,row)_count` and `is_empty`. The first set of methods return the number of columns and rows
  respectively. The method `is_empty` returns if the table is empty (contains no data rows). Implemented by
  [Techassi](https://github.com/Techassi) in [#119](https://github.com/Nukesor/comfy-table/pull/119).

### Chore

- Bump crossterm dependency

## [7.0.1] - 2023-06-16

## Fix

- Fix a panic when working with extreme paddings, where `(padding.left + padding.right) > u16::MAX`.
- Fix a panic when working with extremely long content, where `(content_width + padding) > u16::MAX`.
- Properly enforce lower boundary constraints.
  Previously, "normal" columns were allocated before lower boundaries were respected.
  This could lead to scenarios, where the table would grow beyond the specified size, when there was a lower boundary.
- Fix calculation of column widths for empty columns.
  The minimum content width for a column is `1` char, but the `column_max_content_widths` function on the table returned a `0` width for fully empty columns.
  This resulted in tables becoming larger than specified if there were any empty columns.

## Misc

- Extend property tests, which lead to the discovery some bugs.

## [7.0.0] - 2023-06-06

### Breaking

- The `Color` and `Attribute` enum are no longer re-exported from crossterm by default.
  Previously, when updating comfy-table, crossterm needed to be upgraded as well, since the compile would otherwise fail due to type incompatibilities.

  To fix this, these enums are now mirrored and internally mapped to their crossterm equivalents, which allows us to safely bump crossterm whenever a new version is released.
  This change will only affect you if your projects explicitly use crossterm and comfy-table at the same time **and** feed crossterm's native types into comfy-table.

  If one wants the old behavior for convenience reasons, this can be enabled via a feature flag.
  However, **this is also a opt-in to potential breaking changes on minor/patch versions**.

- Bump minimum version to v1.64

### Added

- `reexport_crossterm` feature flag to enable old crossterm re-export.

## [6.2.0] - 2023-05-26

### Added

- Add support for custom ansi styling inside of cells. This feature is hidden behind the feature flag `custom_styling`. Implemented by [blueforesticarus](https://github.com/blueforesticarus) in [#93](https://github.com/Nukesor/comfy-table/pull/93).
- Add helper functions `add_row[s]_if`, which filtering of rows by a predicate. Implemented by [Techassi](https://github.com/Techassi) in [#106](https://github.com/Nukesor/comfy-table/pull/106).

### Maintenance

- Bump dependencies

## [6.1.4] - 2022-12-31

### Added

- New preset `ASCII_FULL_CONDENSED` [#97](https://github.com/Nukesor/comfy-table/pull/97)

## [6.1.3] - 2022-11-21

### Fixed

- Disable unneeded crossterm `bracketed-paste` feature.

## [6.1.2] - 2022-10-27

### Fixed

- `Table::row_iter` no longer requires a `&mut self`, but only `&self`.

## [6.1.1] - 2022-10-22

### Fixed

- Fixed an issue where dynamic arrangement failed when setting the table to the exact width of the content [#90](https://github.com/Nukesor/comfy-table/issues/90).
- The header size is now properly respected in the final optimization step [#90](https://github.com/Nukesor/comfy-table/issues/90).
  Previously, this wasn't the case and lead to weird formatting behavior when both of the following were true
  - Dynamic content adjustment was active.
  - The table didn't fit into the the available space.
  - The header of a row was longer than its content.
- Fix wrong LowerBoundary calculation. This was introduced in commit bee764d, when this logic was refactored. [#90](https://github.com/Nukesor/comfy-table/issues/90).
- `Table::column_iter` no longer requires a `&mut self`, but only `&self`.

### Added

- Expose current ContentArrangement for table via `table.content_arrangement`.

## [6.1.0] - 2022-08-28

### Added

- Add `Table::add_rows` to add multiple rows at the same time.

### Misc

- Update crossterm to `v0.24`

## [6.0.0] - 2022-05-31

### Added

- Add `Table::style_text_only()`, which prevents non-delimiter whitespaces in cells to be styled.
- Add the `Table::discover_columns` function and add info on when to use it to `Row::add_cell`.

### Breaking Changes

- Renaming of several functions to be Rust idiomatic:
  - `Cell::get_content` -> `Cell::content`
  - `Column::get_padding_width` -> `Column::padding_width`
  - `Column::get_constraint` -> `Column::constraint`
  - `Table::get_header` -> `Table::header`
  - `Table::get_table_width` -> `Table::width`
  - `Table::set_table_width` -> `Table::set_width`
  - `Table::set_style` -> `Table::style`
  - `Table::get_column` -> `Table::column`
  - `Table::get_column_mut` -> `Table::column_mut`
  - `Table::get_row` -> `Table::row`
  - `Table::get_row_mut` -> `Table::row_mut`
- `Column::get_max_width` and `Column::get_max_content_width` have been removed as we cannot guarantee that these numbers are always correct.
  Use `Table::column_max_content_widths` instead

### Changed

- `Table::column_max_content_widths` now performs a full scan of the table's content when called.
- Don't include `Table::is_tty`, `Table::force_no_tty` and `Table::should_style` if `tty` feature isn't enabled.

## [5.0.1] - 2022-02-18

### Updates

- All dependencies have been bumped.

## [5.0.0] - 2021-11-07

### Updates

- All dependencies have been bumped.

### Added

- Add option to use `stderr` for `is_tty` check [#25](https://github.com/Nukesor/comfy-table/pull/47).

### Breaking

- Remove `ASCII_HORIZONTAL_BORDERS_ONLY` in favor of `ASCII_HORIZONTAL_ONLY`.
- Remove `UTF8_HORIZONTAL_BORDERS_ONLY` in favor of `UTF8_HORIZONTAL_ONLY`.

## [4.1.1] - 2021-08-11

### Added

- `tty` feature flag, which enables tty related functionality.
  This includes styling and terminal-width detection.
  The `tty` feature flag is enabled by default.
  Implemented by [roee88](https://github.com/roee88) in [#47](https://github.com/Nukesor/comfy-table/pull/47).

## [4.1.0] - 2021-08-09

### Added

- Add `modifiers::UTF8_SOLID_INNER_BORDERS`, which makes the inner borders solid lines: `│─` by [ModProg](https://github.com/ModProg) for [#39](https://github.com/Nukesor/comfy-table/issues/39).
- Add `presets::ASCII_BORDERS_ONLY_CONDENSED`, which is `ASCII_BORDERS_ONLY` but without spacing between rows [#43](https://github.com/Nukesor/comfy-table/issues/43).

### Fixed

- Several preset examples weren't correct.
- Multi-character UTF8 symbols are now handled correctly in most situations.
  Table-layout might still break for 1-character columns.
- Mid-word splitting now takes multi-character utf8 characters into account.

### Changed

- Rename `ASCII_HORIZONTAL_BORDERS_ONLY` to `ASCII_HORIZONTAL_ONLY`. Old imports will still work until v5.0.
- Rename `UTF8_HORIZONTAL_BORDERS_ONLY` to `UTF8_HORIZONTAL_ONLY`. Old imports will still work until v5.0.

## [4.0.1] - 2021-07-08

### Fixed

- Some docstrings on the `ColumnConstraint` and `Width` enum were wrong.

## [4.0.0] - 2021-07-03

### Added

- Add `Table::lines`, which returns an iterator over all lines of the final table output by [dmaahs2017](https://github.com/dmaahs2017) for [#35](https://github.com/Nukesor/comfy-table/issues/35).
- Add `ColumnConstraints::Boundaries{lower: Width, upper: Width}` to specify both an upper and a lower boundary.

### Fixed

- Fixed percentages sometimes weren't correctly calculated, due to not taking border columns into account.
- Return `None` for `Table::get_table_width`, if getting the terminal size somehow failed.
- Fixed a lot of possible, but highly unlikely number conversion overflow issues.
- Run space optimization under all circumstances.
- Percentage constraints with values of >100 will now be capped to 100.
- The MinConstraint would be ignored when:
  - The content was larger than the min constraint
  - There was less space available than specified in the constraint.

### Changed

- The way ColumnConstraints are initialized has been changed.
  There is now

```
enum ColumnConstraints {
    ...,
    /// Enforce a absolute width for a column.
    Absolute(Width),
    /// Specify a lower boundary, either fixed or as percentage of the total width.
    LowerBoundary(Width),
    /// Specify a upper boundary, either fixed or as percentage of the total width.
    UpperBoundary(Width),
}

pub enum Width {
    /// Specify a min amount of characters per line for a column.
    Fixed(u16),
    /// Set a a minimum percentage in respect to table_width for this column.
    /// Values above 100 will be automatically reduced to 100.
    /// **Warning:** This option will be ignored, if the width cannot be determined!
    Percentage(u16),
}
```

Instead of the old

```
enum ColumnConstraints {
    ...,
    Width(u16),
    MinWidth(u16),
    MaxWidth(u16),
    Percentage(u16),
    MinPercentage(u16),
    MaxPercentage(u16),
}
```

## [3.0.0] - 2021-06-13

### Breaking changes

- Remove most custom traits and replace them with std's generic `From` trait. \
   Check the docs on the trait implementations for Cell, Row and Cells
- Add the `Cells` type, to allow super generic `Iterator -> Row` conversions.

## [2.1.0] - 2021-01-26

### Added

- `DynamicFullWidth` arrangement.
  This mode is basically the same as the `Dynamic` arrangement mode, but it will always use the full available width, even if there isn't enough content to fill the space.

## [2.0.0] - 2021-01-16

### Added

**Dynamic arrangement**

A new logic to optimize space usage after splitting content has been added.\
If there is a lot of unused space after the content has been arranged, this space will now be redistributed to the remaining columns.
Or it will be removed if there are no other columns.

**This is considered a breaking change, since this can result in different table layouts!!**

This process is far from perfect, but the behavior is better than before.

Old behavior:

```
+-----------------------------------+-----------------------------------+------+
| Header1                           | Header2                           | Head |
+==============================================================================+
| This is a very long line with a   | This is text with a               | smol |
| lot of text                       | anotherverylongtexttesttest       |      |
+-----------------------------------+-----------------------------------+------+
```

New behavior:

```
+-----------------------------------------+-----------------------------+------+
| Header1                                 | Header2                     | Head |
+==============================================================================+
| This is a very long line with a lot of  | This is text with a         | smol |
| text                                    | anotherverylongtexttesttest |      |
+-----------------------------------------+-----------------------------+------+
```

Old behavior:

```
+------------------------------------------------+
| Header1                                        |
+================================================+
| This is text with a                            |
| anotherverylongtexttesttestaa                  |
+------------------------------------------------+
```

New behavior:

```
+-------------------------------+
| Header1                       |
+===============================+
| This is text with a           |
| anotherverylongtexttesttestaa |
+-------------------------------+
```

## [1.6.0] - 2021-01-16

### Added

- Add the `NOTHING` preset, which prints no borders or lines at all.

## [1.5.0] - 2020-12-29

### Added

- Add `table::trim_fmt`, which trims all trailing whitespaces on tables with no right border.

## [1.4.0] - 2020-12-06

### Added

- Allow to set custom delimiters on tables, columns and cells.

### Changed

- Expose all important traits. I.e. `ToRow`, `ToCell` and `ToCells`.

## [1.3.0] - 2020-11-20

### Added

- New ColumnConstraint for hiding columns

## [1.2.0] - 2020-10-27

### Added

- Add the option to set a max-height on rows. Long content will be truncated.

## [1.1.1] - 2020-08-23

### Changed

- A simple update of all dependencies.

## [1.1.0] - 2020-08-23

### Changed

- Move `is_tty` logic from `atty` to `crossterm`.
- Remove `skeptic`, since it fails in CI and bloats compile time. Compile time is reduced by ca. 40%.

## [1.0.0] - 2020-07-07

### Changed

- The project has been in use for quite some time and seems to be quite stable!
- Use cargo's `example` functionality for examples.

## [0.1.1] - 2020-05-24

### Added

- `Column::get_max_width()`, which returns the character count of the widest line in this column including padding.
- `current_style_as_preset` method for convenient conversion of a style into a preset
- New Markdown like table style prefix. Thanks to [joeydumont](https://github.com/joeydumont).

## [0.1.0] - 2020-03-21

### Added

- Better test coverage

### Fixed

- Fixed a bug with broken percentage constraints for super wide tables.

## [0.0.7] - 2020-02-09

### Added

- Introduce proptest

### Fixed

- Fix wrong calculation due to bytes count instead of char count
- Fix panics caused by wrong string splits

## [0.0.6] - 2020-01-31

### Changed

- Crossterm 0.15 update
- Simplify the project's module structure

## [0.0.5] - 2020-01-29

### Added

- Add `Column::remove_constraint()`
- Preset `UTF8_NO_BORDERS`
- Preset `UTF8_HORIZONTAL_BORDERS_ONLY`

## [0.0.4] - 2020-01-27

### Added

- Add skeptic tests
- Add code coverage
- A lot more tests

### Changed

- Removed `Hidden` Constraint
