use crate::{compilers::ParsedSource, Graph};
use std::{collections::HashSet, io, io::Write, str::FromStr};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charset {
    // when operating in a console on windows non-UTF-8 byte sequences are not supported on
    // stdout, See also [`StdoutLock`]
    #[cfg_attr(not(target_os = "windows"), default)]
    Utf8,
    #[cfg_attr(target_os = "windows", default)]
    Ascii,
}

impl FromStr for Charset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "utf8" => Ok(Self::Utf8),
            "ascii" => Ok(Self::Ascii),
            s => Err(format!("invalid charset: {s}")),
        }
    }
}

/// Options to configure formatting
#[derive(Clone, Debug, Default)]
pub struct TreeOptions {
    /// The style of characters to use.
    pub charset: Charset,
    /// If `true`, duplicate imports will be repeated.
    /// If `false`, duplicates are suffixed with `(*)`, and their imports
    /// won't be shown.
    pub no_dedupe: bool,
}

/// Internal helper type for symbols
struct Symbols {
    down: &'static str,
    tee: &'static str,
    ell: &'static str,
    right: &'static str,
}

static UTF8_SYMBOLS: Symbols = Symbols { down: "│", tee: "├", ell: "└", right: "─" };

static ASCII_SYMBOLS: Symbols = Symbols { down: "|", tee: "|", ell: "`", right: "-" };

pub fn print<D: ParsedSource>(
    graph: &Graph<D>,
    opts: &TreeOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    let symbols = match opts.charset {
        Charset::Utf8 => &UTF8_SYMBOLS,
        Charset::Ascii => &ASCII_SYMBOLS,
    };

    // used to determine whether to display `(*)`
    let mut visited_imports = HashSet::new();

    // A stack of bools used to determine where | symbols should appear
    // when printing a line.
    let mut levels_continue = Vec::new();
    // used to detect dependency cycles when --no-dedupe is used.
    // contains a `Node` for each level.
    let mut write_stack = Vec::new();

    for (node_index, _) in graph.input_nodes().enumerate() {
        print_node(
            graph,
            node_index,
            symbols,
            opts.no_dedupe,
            &mut visited_imports,
            &mut levels_continue,
            &mut write_stack,
            out,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn print_node<D: ParsedSource>(
    graph: &Graph<D>,
    node_index: usize,
    symbols: &Symbols,
    no_dedupe: bool,
    visited_imports: &mut HashSet<usize>,
    levels_continue: &mut Vec<bool>,
    write_stack: &mut Vec<usize>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let new_node = no_dedupe || visited_imports.insert(node_index);

    if let Some((last_continues, rest)) = levels_continue.split_last() {
        for continues in rest {
            let c = if *continues { symbols.down } else { " " };
            write!(out, "{c}   ")?;
        }

        let c = if *last_continues { symbols.tee } else { symbols.ell };
        write!(out, "{0}{1}{1} ", c, symbols.right)?;
    }

    let in_cycle = write_stack.contains(&node_index);
    // if this node does not have any outgoing edges, don't include the (*)
    // since there isn't really anything "deduplicated", and it generally just
    // adds noise.
    let has_deps = graph.has_outgoing_edges(node_index);
    let star = if (new_node && !in_cycle) || !has_deps { "" } else { " (*)" };

    writeln!(out, "{}{star}", graph.display_node(node_index))?;

    if !new_node || in_cycle {
        return Ok(());
    }
    write_stack.push(node_index);

    print_imports(
        graph,
        node_index,
        symbols,
        no_dedupe,
        visited_imports,
        levels_continue,
        write_stack,
        out,
    )?;

    write_stack.pop();

    Ok(())
}

/// Prints all the imports of a node
#[allow(clippy::too_many_arguments)]
fn print_imports<D: ParsedSource>(
    graph: &Graph<D>,
    node_index: usize,
    symbols: &Symbols,
    no_dedupe: bool,
    visited_imports: &mut HashSet<usize>,
    levels_continue: &mut Vec<bool>,
    write_stack: &mut Vec<usize>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let imports = graph.imported_nodes(node_index);
    if imports.is_empty() {
        return Ok(());
    }

    let mut iter = imports.iter().peekable();

    while let Some(import) = iter.next() {
        levels_continue.push(iter.peek().is_some());
        print_node(
            graph,
            *import,
            symbols,
            no_dedupe,
            visited_imports,
            levels_continue,
            write_stack,
            out,
        )?;
        levels_continue.pop();
    }

    Ok(())
}
