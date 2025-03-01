use clipboard_win::raw::{register_format, format_name, format_name_big};

#[test]
fn custom_format_smol() {
    const NAME: &str = "SMOL";

    let format = register_format(NAME).expect("To create format").get();

    let mut buf = [0u8; 4];
    let name = format_name(format, buf.as_mut_slice().into()).expect("To get name");

    assert_eq!(NAME, name);
}

#[test]
fn custom_format_big() {
    const NAME: &str = "ahdkajhfdsakjfhhdsakjgfdsakjgfdsakjghrdskjghfdskjghrdskjghfdkjghfds;kjghfd;kjgfdsjgfdskjgbfdkjgfdgkjfdsahgkjfdghkjfdgkjfdgfdkjgbfdkjgsakjdhsakjdhs";

    let format = match register_format(NAME) {
        Some(format) => format.get(),
        None => {
            panic!("Failed to register format: {}", std::io::Error::last_os_error());
        },
    };

    let name = format_name_big(format).expect("To get name");

    assert_eq!(NAME, name.as_str());
}

#[test]
fn custom_format_overflow() {
    const BUF_SIZE: usize = 128;
    const NAME: &str = "ahdkajhfdsakjfhhdsakjgfdsakjgfdsakjghrdskjghfdskjghrdskjghfdkjghfds;kjghfd;kjgfdsjgfdskjgbfdkjgfdgkjfdsahgkjfdghkjfdgkjfdgfdkjgbfdkjgsakjdhsakjdhs";

    let format = match register_format(NAME) {
        Some(format) => format.get(),
        None => {
            panic!("Failed to register format: {}", std::io::Error::last_os_error());
        },
    };

    let mut buf = [0u8; BUF_SIZE];
    let name = format_name(format, buf.as_mut_slice().into());
    assert!(name.is_none());
}

#[test]
fn custom_format_trunc_default() {
    const BUF_SIZE: usize = 5;
    let mut buf = [0u8; BUF_SIZE];
    let name = format_name(clipboard_win::formats::CF_TEXT, buf.as_mut_slice().into()).expect("to get CF_TEXT");
    assert_eq!(name, "CF_TE");
}

#[test]
fn custom_format_default_up_to_buf_capacity() {
    const BUF_SIZE: usize = 7;
    let mut buf = [0u8; BUF_SIZE];
    let name = format_name(clipboard_win::formats::CF_TEXT, buf.as_mut_slice().into()).expect("to get CF_TEXT");
    assert_eq!(name, "CF_TEXT");
}

#[test]
fn custom_format_with_wide_chars() {
    const BUF_SIZE: usize = 8;
    const NAME: &str = "一番";

    let format = match register_format(NAME) {
        Some(format) => format.get(),
        None => {
            panic!("Failed to register format: {}", std::io::Error::last_os_error());
        },
    };

    let mut buf = [0u8; BUF_SIZE];
    let name = format_name(format, buf.as_mut_slice().into()).expect("to get format");
    assert_eq!(name, "一番");
}
