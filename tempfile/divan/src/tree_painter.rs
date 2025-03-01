//! Happy little trees.

use std::{io::Write, iter::repeat};

use crate::{
    alloc::{AllocOp, AllocTally},
    counter::{AnyCounter, BytesFormat, KnownCounterKind},
    stats::{Stats, StatsSet},
    util,
};

const TREE_COL_BUF: usize = 2;

/// Paints tree-style output using box-drawing characters.
pub(crate) struct TreePainter {
    /// The maximum number of characters taken by a name and its prefix. Emitted
    /// information should be left-padded to start at this column.
    max_name_span: usize,

    column_widths: [usize; TreeColumn::COUNT],

    depth: usize,

    /// The current prefix to the name and content, e.g.
    /// <code>│     │  </code> for three levels of nesting with the second level
    /// being on the last node.
    current_prefix: String,

    /// Buffer for writing to before printing to stdout.
    write_buf: String,
}

impl TreePainter {
    pub fn new(max_name_span: usize, column_widths: [usize; TreeColumn::COUNT]) -> Self {
        Self {
            max_name_span,
            column_widths,
            depth: 0,
            current_prefix: String::new(),
            write_buf: String::new(),
        }
    }
}

impl TreePainter {
    /// Enter a parent node.
    pub fn start_parent(&mut self, name: &str, is_last: bool) {
        let is_top_level = self.depth == 0;
        let has_columns = self.has_columns();

        let buf = &mut self.write_buf;
        buf.clear();

        let branch = if is_top_level {
            ""
        } else if !is_last {
            "├─ "
        } else {
            "╰─ "
        };
        buf.extend([self.current_prefix.as_str(), branch, name]);

        // Right-pad name if `has_columns`
        if has_columns {
            let max_span = self.max_name_span;
            let buf_len = buf.chars().count();
            let pad_len = TREE_COL_BUF + max_span.saturating_sub(buf_len);
            buf.extend(repeat(' ').take(pad_len));

            if buf_len > max_span {
                self.max_name_span = buf_len;
            }
        }

        // Write column headings.
        if has_columns && is_top_level {
            let names = TreeColumnData::from_fn(TreeColumn::name);
            names.write(buf, &mut self.column_widths);
        }

        // Write column spacers.
        if has_columns && !is_top_level {
            TreeColumnData([""; TreeColumn::COUNT]).write(buf, &mut self.column_widths);
        }

        println!("{buf}");

        self.depth += 1;

        if !is_top_level {
            self.current_prefix.push_str(if !is_last { "│  " } else { "   " });
        }
    }

    /// Exit the current parent node.
    pub fn finish_parent(&mut self) {
        self.depth -= 1;

        // Improve legibility for multiple top-level parents.
        if self.depth == 0 {
            println!();
        }

        // The prefix is extended by 3 `char`s at a time.
        let new_prefix_len = {
            let mut iter = self.current_prefix.chars();
            _ = iter.by_ref().rev().nth(2);
            iter.as_str().len()
        };
        self.current_prefix.truncate(new_prefix_len);
    }

    /// Indicate that the next child node was ignored.
    ///
    /// This semantically combines start/finish operations.
    pub fn ignore_leaf(&mut self, name: &str, is_last: bool) {
        let has_columns = self.has_columns();

        let buf = &mut self.write_buf;
        buf.clear();

        let branch = if !is_last { "├─ " } else { "╰─ " };
        buf.extend([self.current_prefix.as_str(), branch, name]);

        right_pad_buffer(buf, &mut self.max_name_span);

        if has_columns {
            TreeColumnData::from_first("(ignored)").write(buf, &mut self.column_widths);
        } else {
            buf.push_str("(ignored)");
        }

        println!("{buf}");
    }

    /// Enter a leaf node.
    pub fn start_leaf(&mut self, name: &str, is_last: bool) {
        let has_columns = self.has_columns();

        let buf = &mut self.write_buf;
        buf.clear();

        let branch = if !is_last { "├─ " } else { "╰─ " };
        buf.extend([self.current_prefix.as_str(), branch, name]);

        // Right-pad buffer if this leaf will have info displayed.
        if has_columns {
            let max_span = self.max_name_span;
            let buf_len = buf.chars().count();
            let pad_len = TREE_COL_BUF + max_span.saturating_sub(buf_len);
            buf.extend(repeat(' ').take(pad_len));

            if buf_len > max_span {
                self.max_name_span = buf_len;
            }
        }

        print!("{buf}");
        _ = std::io::stdout().flush();
    }

    /// Exit the current leaf node.
    pub fn finish_empty_leaf(&mut self) {
        println!();
    }

    /// Exit the current leaf node, emitting statistics.
    pub fn finish_leaf(&mut self, is_last: bool, stats: &Stats, bytes_format: BytesFormat) {
        let prep_buffer = |buf: &mut String, max_span: &mut usize| {
            buf.clear();
            buf.push_str(&self.current_prefix);

            if !is_last {
                buf.push('│');
            }

            right_pad_buffer(buf, max_span);
        };

        let buf = &mut self.write_buf;
        buf.clear();

        // Serialize max alloc counts and sizes early so we can resize columns
        // early.
        let serialized_max_alloc_counts = if stats.max_alloc.size.is_zero() {
            None
        } else {
            Some(TreeColumn::ALL.map(|column| {
                let Some(&max_alloc_count) = column.get_stat(&stats.max_alloc.count) else {
                    return String::new();
                };

                let prefix = if column.is_first() { "  " } else { "" };
                format!("{prefix}{}", util::fmt::format_f64(max_alloc_count, 4))
            }))
        };

        let serialized_max_alloc_sizes = if stats.max_alloc.size.is_zero() {
            None
        } else {
            Some(TreeColumn::ALL.map(|column| {
                let Some(&max_alloc_size) = column.get_stat(&stats.max_alloc.size) else {
                    return String::new();
                };

                let prefix = if column.is_first() { "  " } else { "" };
                format!("{prefix}{}", util::fmt::format_bytes(max_alloc_size, 4, bytes_format))
            }))
        };

        // Serialize alloc tallies early so we can resize columns early.
        let serialized_alloc_tallies = AllocOp::ALL.map(|op| {
            let tally = stats.alloc_tallies.get(op);

            if tally.is_zero() {
                return None;
            }

            let column_tallies = TreeColumn::ALL.map(|column| {
                let prefix = if column.is_first() { "  " } else { "" };

                let tally = AllocTally {
                    count: column.get_stat(&tally.count).copied()?,
                    size: column.get_stat(&tally.size).copied()?,
                };

                Some((prefix, tally))
            });

            Some(AllocTally {
                count: column_tallies.map(|tally| {
                    if let Some((prefix, tally)) = tally {
                        format!("{prefix}{}", util::fmt::format_f64(tally.count, 4))
                    } else {
                        String::new()
                    }
                }),
                size: column_tallies.map(|tally| {
                    if let Some((prefix, tally)) = tally {
                        format!("{prefix}{}", util::fmt::format_bytes(tally.size, 4, bytes_format))
                    } else {
                        String::new()
                    }
                }),
            })
        });

        // Serialize counter stats early so we can resize columns early.
        let serialized_counters = KnownCounterKind::ALL.map(|counter_kind| {
            let counter_stats = stats.get_counts(counter_kind);

            TreeColumn::ALL
                .map(|column| -> Option<String> {
                    let count = *column.get_stat(counter_stats?)?;
                    let time = *column.get_stat(&stats.time)?;

                    Some(
                        AnyCounter::known(counter_kind, count)
                            .display_throughput(time, bytes_format)
                            .to_string(),
                    )
                })
                .map(Option::unwrap_or_default)
        });

        // Set column widths based on serialized strings.
        for column in TreeColumn::time_stats() {
            let width = &mut self.column_widths[column as usize];

            let mut update_width = |s: &str| {
                *width = (*width).max(s.chars().count());
            };

            for counter in &serialized_counters {
                update_width(&counter[column as usize]);
            }

            let serialized_max_alloc_counts = serialized_max_alloc_counts.iter().flatten();
            let serialized_max_alloc_sizes = serialized_max_alloc_sizes.iter().flatten();
            for s in serialized_max_alloc_counts.chain(serialized_max_alloc_sizes) {
                update_width(s);
            }

            for s in serialized_alloc_tallies
                .iter()
                .flatten()
                .flat_map(AllocTally::as_array)
                .map(|values| &values[column as usize])
            {
                update_width(s);
            }
        }

        // Write time stats with iter and sample counts.
        TreeColumnData::from_fn(|column| -> String {
            let stat: &dyn ToString = match column {
                TreeColumn::Fastest => &stats.time.fastest,
                TreeColumn::Slowest => &stats.time.slowest,
                TreeColumn::Median => &stats.time.median,
                TreeColumn::Mean => &stats.time.mean,
                TreeColumn::Samples => &stats.sample_count,
                TreeColumn::Iters => &stats.iter_count,
            };
            stat.to_string()
        })
        .as_ref::<str>()
        .write(buf, &mut self.column_widths);

        println!("{buf}");

        // Write counter stats.
        let counter_stats = serialized_counters.map(TreeColumnData);
        for counter_kind in KnownCounterKind::ALL {
            let counter_stats = counter_stats[counter_kind as usize].as_ref::<str>();

            // Skip empty rows.
            if counter_stats.0.iter().all(|s| s.is_empty()) {
                continue;
            }

            prep_buffer(buf, &mut self.max_name_span);

            counter_stats.write(buf, &mut self.column_widths);
            println!("{buf}");
        }

        // Write max allocated bytes.
        if serialized_max_alloc_counts.is_some() || serialized_max_alloc_sizes.is_some() {
            prep_buffer(buf, &mut self.max_name_span);

            TreeColumnData::from_first("max alloc:").write(buf, &mut self.column_widths);
            println!("{buf}");

            for serialized in
                [serialized_max_alloc_counts.as_ref(), serialized_max_alloc_sizes.as_ref()]
                    .into_iter()
                    .flatten()
            {
                prep_buffer(buf, &mut self.max_name_span);

                TreeColumnData::from_fn(|column| serialized[column as usize].as_str())
                    .write(buf, &mut self.column_widths);

                println!("{buf}");
            }
        }

        // Write allocation tallies.
        for op in [AllocOp::Alloc, AllocOp::Dealloc, AllocOp::Grow, AllocOp::Shrink] {
            let Some(tallies) = &serialized_alloc_tallies[op as usize] else {
                continue;
            };

            prep_buffer(buf, &mut self.max_name_span);

            TreeColumnData::from_first(op.prefix()).write(buf, &mut self.column_widths);
            println!("{buf}");

            for value in tallies.as_array() {
                prep_buffer(buf, &mut self.max_name_span);

                TreeColumnData::from_fn(|column| value[column as usize].as_str())
                    .write(buf, &mut self.column_widths);

                println!("{buf}");
            }
        }
    }

    fn has_columns(&self) -> bool {
        !self.column_widths.iter().all(|&w| w == 0)
    }
}

/// Columns of the table next to the tree.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeColumn {
    Fastest,
    Slowest,
    Median,
    Mean,
    Samples,
    Iters,
}

impl TreeColumn {
    pub const COUNT: usize = 6;

    pub const ALL: [Self; Self::COUNT] = {
        use TreeColumn::*;
        [Fastest, Slowest, Median, Mean, Samples, Iters]
    };

    #[inline]
    pub fn time_stats() -> impl Iterator<Item = Self> {
        use TreeColumn::*;
        [Fastest, Slowest, Median, Mean].into_iter()
    }

    #[inline]
    pub fn is_first(self) -> bool {
        let [first, ..] = Self::ALL;
        self == first
    }

    #[inline]
    pub fn is_last(self) -> bool {
        let [.., last] = Self::ALL;
        self == last
    }

    fn name(self) -> &'static str {
        match self {
            Self::Fastest => "fastest",
            Self::Slowest => "slowest",
            Self::Median => "median",
            Self::Mean => "mean",
            Self::Samples => "samples",
            Self::Iters => "iters",
        }
    }

    #[inline]
    pub fn is_time_stat(self) -> bool {
        use TreeColumn::*;
        matches!(self, Fastest | Slowest | Median | Mean)
    }

    #[inline]
    fn get_stat<T>(self, stats: &StatsSet<T>) -> Option<&T> {
        match self {
            Self::Fastest => Some(&stats.fastest),
            Self::Slowest => Some(&stats.slowest),
            Self::Median => Some(&stats.median),
            Self::Mean => Some(&stats.mean),
            Self::Samples | Self::Iters => None,
        }
    }
}

#[derive(Default)]
struct TreeColumnData<T>([T; TreeColumn::COUNT]);

impl<T> TreeColumnData<T> {
    #[inline]
    fn from_first(value: T) -> Self
    where
        Self: Default,
    {
        let mut data = Self::default();
        data.0[0] = value;
        data
    }

    #[inline]
    fn from_fn<F>(f: F) -> Self
    where
        F: FnMut(TreeColumn) -> T,
    {
        Self(TreeColumn::ALL.map(f))
    }
}

impl TreeColumnData<&str> {
    /// Writes the column data into the buffer.
    fn write(&self, buf: &mut String, column_widths: &mut [usize; TreeColumn::COUNT]) {
        for (column, value) in self.0.iter().enumerate() {
            let is_first = column == 0;
            let is_last = column == TreeColumn::COUNT - 1;

            let value_width = value.chars().count();

            // Write separator.
            if !is_first {
                let mut sep = " │ ";

                // Prevent trailing spaces.
                if is_last && value_width == 0 {
                    sep = &sep[..sep.len() - 1];
                };

                buf.push_str(sep);
            }

            buf.push_str(value);

            // Right-pad remaining width or update column width to new maximum.
            if !is_last {
                if let Some(rem_width) = column_widths[column].checked_sub(value_width) {
                    buf.extend(repeat(' ').take(rem_width));
                } else {
                    column_widths[column] = value_width;
                }
            }
        }
    }
}

impl<T> TreeColumnData<T> {
    #[inline]
    fn as_ref<U: ?Sized>(&self) -> TreeColumnData<&U>
    where
        T: AsRef<U>,
    {
        TreeColumnData::from_fn(|column| self.0[column as usize].as_ref())
    }
}

fn right_pad_buffer(buf: &mut String, max_span: &mut usize) {
    let buf_len = buf.chars().count();
    let pad_len = TREE_COL_BUF + max_span.saturating_sub(buf_len);
    buf.extend(repeat(' ').take(pad_len));

    if buf_len > *max_span {
        *max_span = buf_len;
    }
}
