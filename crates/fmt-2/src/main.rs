use std::{io::Read, path::PathBuf};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let (src, path) = if args.len() < 2 {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).unwrap();
        (s, None)
    } else {
        let path = PathBuf::from(&args[1]);
        (std::fs::read_to_string(&path).unwrap(), Some(path))
    };
    match forge_fmt_2::printer::format_source(&src, path.as_deref(), Default::default()) {
        Ok(formatted) => {
            print!("{formatted}");
        }
        Err(e) => {
            eprintln!("failed formatting: {e}");
            std::process::exit(1);
        }
    }
}
