#[macro_export]
macro_rules! test_debug {
    ($($args:tt)*) => {
        $crate::test_debug(format_args!($($args)*))
    }
}

#[macro_export]
macro_rules! test_trace {
    ($($args:tt)*) => {
        $crate::test_trace(format_args!($($args)*))
    }
}
