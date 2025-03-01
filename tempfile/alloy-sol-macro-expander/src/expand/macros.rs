#[cfg(feature = "json")]
macro_rules! if_json {
    ($($t:tt)*) => { $($t)* };
}

#[cfg(not(feature = "json"))]
macro_rules! if_json {
    ($($t:tt)*) => {
        crate::expand::emit_json_error();
        TokenStream::new()
    };
}
