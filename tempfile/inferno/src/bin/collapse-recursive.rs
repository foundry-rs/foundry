use std::io;
use std::path::PathBuf;

use clap::Parser;
use inferno::collapse::recursive::{Folder, Options};
use inferno::collapse::{Collapse, DEFAULT_NTHREADS};
use once_cell::sync::Lazy;

static NTHREADS: Lazy<String> = Lazy::new(|| DEFAULT_NTHREADS.to_string());

#[derive(Debug, Parser)]
#[clap(name = "inferno-collapse-recursive", about)]
struct Opt {
    /// Number of threads to use
    #[clap(
        short = 'n',
        long = "nthreads",
        default_value = &**NTHREADS,
        value_name = "UINT"
    )]
    nthreads: usize,

    #[clap(value_name = "PATH")]
    /// Collapse output file, or STDIN if not specified
    infile: Option<PathBuf>,
}

impl Opt {
    fn into_parts(self) -> (Option<PathBuf>, Options) {
        let mut options = Options::default();
        options.nthreads = self.nthreads;
        (self.infile, options)
    }
}

fn main() -> io::Result<()> {
    let opt = Opt::parse();
    let (infile, options) = opt.into_parts();
    Folder::from(options).collapse_file_to_stdout(infile.as_ref())
}
