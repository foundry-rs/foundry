use crate::{Cmd, Shell};
use structopt::{clap::AppSettings, StructOpt};

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[AppSettings::ColoredHelp])]
pub enum Args {
    #[structopt(about = "Lists all contract names.")]
    Contracts {
        #[structopt(help = "Print all contract names from the project matching that name.")]
        name: Option<String>,
        #[structopt(
            help = "Print all contract names from every project.",
            long,
            short,
            conflicts_with = "name"
        )]
        all: bool,
    },
}

impl Cmd for Args {
    fn run(self, shell: &mut Shell) -> eyre::Result<()> {
        match self {
            Args::Contracts { name, all } => {
                if all {
                    for artifacts in shell.session.artifacts.values() {
                        for name in artifacts.keys() {
                            println!("{}", name);
                        }
                    }
                } else {
                    if let Some(artifacts) = name
                        .as_ref()
                        .or(shell.workspace())
                        .map(|name| shell.session.artifacts.get(name))
                        .flatten()
                    {
                        for name in artifacts.keys() {
                            println!("{}", name);
                        }
                    } else {
                        println!(".");
                    }
                }
            }
        }

        Ok(())
    }
}
