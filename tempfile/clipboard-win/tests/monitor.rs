use clipboard_win::{Clipboard, Monitor, set_clipboard_string};

#[test]
fn should_get_clipboard_event() {
    let mut monitor = Monitor::new().expect("create monitor");
    let result = monitor.try_recv().expect("Success");
    assert!(!result);

    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");
    set_clipboard_string("test").expect("Success");
    let result = monitor.try_recv().expect("Success");
    assert!(result);
    let result = monitor.try_recv().expect("Success");
    assert!(!result);

    monitor.shutdown_channel();
    set_clipboard_string("test").expect("Success");
    let result = monitor.try_recv().expect("Success");
    assert!(result);
    let result = monitor.try_recv().expect("Success");
    assert!(!result);

    set_clipboard_string("test").expect("Success");
    let result = monitor.recv().expect("Success");
    assert!(result);
    monitor.shutdown_channel();
    let result = monitor.recv().expect("Success");
    assert!(!result);
}
