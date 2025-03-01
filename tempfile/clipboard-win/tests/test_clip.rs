use clipboard_win::{Getter, Setter, Clipboard, is_format_avail, types};
use clipboard_win::raw::which_format_avail;
use clipboard_win::formats::{Html, RawData, Unicode, Bitmap, CF_TEXT, CF_UNICODETEXT, CF_BITMAP, FileList, CF_HDROP};

fn should_set_file_list() {
    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");
    // Note that you will not be able to paste the paths below in Windows Explorer because Explorer
    // does not play nice with canonicalize: https://github.com/rust-lang/rust/issues/42869.
    // Pasting in Explorer works fine with regular, non-UNC paths.
    let paths = [
        std::fs::canonicalize("tests/test-image.bmp").expect("to get abs path").display().to_string(),
        std::fs::canonicalize("tests/formats.rs").expect("to get abs path").display().to_string(),
    ];
    FileList.write_clipboard(&paths).expect("set file to copy");

    let mut set_files = Vec::<String>::with_capacity(2);
    FileList.read_clipboard(&mut set_files).expect("read");
    assert_eq!(set_files, paths);
}

fn should_work_with_bitmap() {
    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");

    let test_image_bytes = std::fs::read("tests/test-image.bmp").expect("Read test image");
    Bitmap.write_clipboard(&test_image_bytes).expect("To set image");

    let mut out = Vec::new();

    assert_eq!(Bitmap.read_clipboard(&mut out).expect("To get image"), out.len());

    assert_eq!(test_image_bytes.len(), out.len());
    assert!(test_image_bytes == out);
}

fn should_work_with_string() {
    let text = "For my waifu\n!";

    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");

    Unicode.write_clipboard(&text).expect("Write text");

    let first_format = which_format_avail(&[CF_TEXT, CF_UNICODETEXT]).unwrap();
    assert!(is_format_avail(CF_UNICODETEXT));
    assert_eq!(CF_UNICODETEXT, first_format.get());

    let mut output = String::new();

    assert_eq!(Unicode.read_clipboard(&mut output).expect("Read text"), text.len());
    assert_eq!(text, output);

    assert_eq!(Unicode.read_clipboard(&mut output).expect("Read text"), text.len());
    assert_eq!(format!("{0}{0}", text), output);
}

fn should_work_with_wide_string() {
    let text = "メヒーシャ!";

    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");

    Unicode.write_clipboard(&text).expect("Write text");

    let mut output = String::new();

    assert_eq!(Unicode.read_clipboard(&mut output).expect("Read text"), text.len());
    assert_eq!(text, output);

    assert_eq!(Unicode.read_clipboard(&mut output).expect("Read text"), text.len());
    assert_eq!(format!("{0}{0}", text), output);
}

fn should_work_with_bytes() {
    let text = "Again waifu!?\0";

    let ascii = RawData(CF_TEXT);
    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");

    ascii.write_clipboard(&text).expect("Write ascii");

    let mut output = String::with_capacity(text.len() * 2);

    {
        let output = unsafe { output.as_mut_vec() };
        assert_eq!(ascii.read_clipboard(output).expect("read ascii"), text.len());
    }

    assert_eq!(text, output);

    {
        let output = unsafe { output.as_mut_vec() };
        assert_eq!(ascii.read_clipboard(output).expect("read ascii"), text.len());
    }

    assert_eq!(format!("{0}{0}", text), output);
}

fn should_work_with_set_empty_string() {
    let text = "";

    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");

    Unicode.write_clipboard(&text).expect("Write text");

    let mut output = String::new();

    assert_eq!(Unicode.read_clipboard(&mut output).expect("Read text"), text.len());
    assert_eq!(text, output);
}

extern "system" {
    fn GetConsoleWindow() -> types::HWND;
}

fn should_set_owner() {
    {
        assert!(clipboard_win::get_owner().is_none());
        let _clip = Clipboard::new_attempts(10).expect("Open clipboard");
        assert!(clipboard_win::get_owner().is_none());
    }

    let console = unsafe { GetConsoleWindow() };
    if !console.is_null() {
        let _clip = Clipboard::new_attempts_for(console, 10).expect("Open clipboard");
        let _ = clipboard_win::empty(); //empty is necessary to finalize association
        assert_eq!(clipboard_win::get_owner().expect("to have owner").as_ptr() as usize, console as usize);
    }
}

fn should_set_get_html() {
    const HTML: &str = "<tr>1</tr>";
    let html1 = Html::new().expect("Create html1");
    let html2 = Html::new().expect("Create html2");
    assert_eq!(html1.code(), html2.code());

    assert!(!is_format_avail(html1.code()));
    assert!(which_format_avail(&[html1.code()]).is_none());
    let _clip = Clipboard::new_attempts(10).expect("Open clipboard");
    html1.write_clipboard(&HTML).expect("write clipboard");

    assert!(is_format_avail(html1.code()));
    assert_eq!(which_format_avail(&[html1.code(), CF_TEXT]).unwrap().get(), html1.code());
    //This works on my PC, but not in CI, wtf MS
    //assert_eq!(which_format_avail(&[CF_TEXT, html1.code()]).unwrap().get(), html1.code());

    let mut out = String::new();
    html1.read_clipboard(&mut out).expect("read clipboard");
    assert_eq!(out, HTML);

    //Check empty output works
    html1.write_clipboard(&"").expect("write clipboard");
    assert!(is_format_avail(html1.into()));

    out.clear();
    html1.read_clipboard(&mut out).expect("read clipboard");
    assert!(out.is_empty());
}

macro_rules! run {
    ($name:ident) => {
        println!("Clipboard test: {}...", stringify!($name));
        $name();
    }
}

#[test]
fn clipboard_should_work() {

    run!(should_work_with_bitmap);
    assert!(is_format_avail(CF_BITMAP));
    run!(should_work_with_string);
    assert!(is_format_avail(CF_UNICODETEXT));
    run!(should_set_file_list);
    assert!(is_format_avail(CF_HDROP));
    run!(should_work_with_wide_string);
    run!(should_work_with_bytes);
    run!(should_work_with_set_empty_string);
    run!(should_set_owner);
    run!(should_set_get_html);
}
