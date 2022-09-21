macro_rules! writeln_doc {
    ($dst:expr, $arg:expr) => {
        writeln_doc!($dst, "{}", $arg)
    };
    ($dst:expr, $format:literal, $($arg:expr),*) => {
        writeln!($dst, "{}", format_args!($format, $($arg.doc(),)*))
    };
}

macro_rules! writeln_code {
    ($dst:expr, $arg:expr) => {
        writeln_code!($dst, "{}", $arg)
    };
    ($dst:expr, $format:literal, $($arg:expr),*) => {
        writeln!($dst, "{}", $crate::output::DocOutput::CodeBlock("solidity", &format!($format, $($arg.as_code(),)*)))
    };
}

pub(crate) use writeln_code;
pub(crate) use writeln_doc;
