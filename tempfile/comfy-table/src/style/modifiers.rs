/// A modifier, that when applied will convert the outer corners to round corners.
/// ```text
/// ╭───────┬───────╮
/// │ Hello │ there │
/// ╞═══════╪═══════╡
/// │ a     ┆ b     │
/// ├╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌┤
/// │ c     ┆ d     │
/// ╰───────┴───────╯
/// ```
pub const UTF8_ROUND_CORNERS: &str = "               ╭╮╰╯";

/// A modifier, that when applied will convert the inner borders to solid lines.
/// ```text
/// ╭───────┬───────╮
/// │ Hello │ there │
/// ╞═══════╪═══════╡
/// │ a     │ b     │
/// ├───────┼───────┤
/// │ c     │ d     │
/// ╰───────┴───────╯
/// ```
pub const UTF8_SOLID_INNER_BORDERS: &str = "        │─         ";
