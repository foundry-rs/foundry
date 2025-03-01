use backtrace::Backtrace;

// This test only works on platforms which have a working `symbol_address`
// function for frames which reports the starting address of a symbol. As a
// result it's only enabled on a few platforms.
const ENABLED: bool = cfg!(all(
    // Windows hasn't really been tested, and macOS doesn't support actually
    // finding an enclosing frame, so disable this
    target_os = "linux",
    // On ARM finding the enclosing function is simply returning the ip itself.
    not(target_arch = "arm"),
));

#[test]
fn backtrace_new_unresolved_should_start_with_call_site_trace() {
    if !ENABLED {
        return;
    }
    let mut b = Backtrace::new_unresolved();
    b.resolve();
    println!("{b:?}");

    assert!(!b.frames().is_empty());

    let this_ip = backtrace_new_unresolved_should_start_with_call_site_trace as usize;
    println!("this_ip: {:?}", this_ip as *const usize);
    let frame_ip = b.frames().first().unwrap().symbol_address() as usize;
    assert_eq!(this_ip, frame_ip);
}

#[test]
fn backtrace_new_should_start_with_call_site_trace() {
    if !ENABLED {
        return;
    }
    let b = Backtrace::new();
    println!("{b:?}");

    assert!(!b.frames().is_empty());

    let this_ip = backtrace_new_should_start_with_call_site_trace as usize;
    let frame_ip = b.frames().first().unwrap().symbol_address() as usize;
    assert_eq!(this_ip, frame_ip);
}
