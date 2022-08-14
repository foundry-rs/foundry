macro_rules! writeln_doc {
    ($dst:expr, $arg:expr) => {
        writeln_doc!($dst, "{}", $arg)
    };
    ($dst:expr, $format:literal, $($arg:expr),*) => {
        writeln!($dst, "{}", format_args!($format, $($arg.doc(),)*))
    };
}

pub(crate) use writeln_doc;
