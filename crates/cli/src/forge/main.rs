use clap::{CommandFactory, Parser};
use clap_complete::generate;
use foundry_cli::{handler, utils, utils::Cmd};

mod cmd;
mod opts;

use cmd::{cache::CacheSubcommands, generate::GenerateSubcommands, watch};
use opts::{Opts, Subcommands};

fn main() -> eyre::Result<()> {
    utils::load_dotenv();
    handler::install()?;
    utils::subscriber();
    utils::enable_paint();

    let opts = Opts::parse();
    match opts.sub {
        Subcommands::Test(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_test(cmd))
            } else {
                let outcome = utils::block_on(cmd.run())?;
                outcome.ensure_ok()
            }
        }
        Subcommands::Script(cmd) => {
            // install the shell before executing the command
            foundry_common::shell::set_shell(foundry_common::shell::Shell::from_args(
                cmd.opts.args.silent,
                cmd.json,
            ))?;
            utils::block_on(cmd.run_script(Default::default()))
        }
        Subcommands::Coverage(cmd) => utils::block_on(cmd.run()),
        Subcommands::Bind(cmd) => cmd.run(),
        Subcommands::Build(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_build(cmd))
            } else {
                cmd.run().map(|_| ())
            }
        }
        Subcommands::Debug(cmd) => utils::block_on(cmd.debug(Default::default())),
        Subcommands::VerifyContract(args) => utils::block_on(args.run()),
        Subcommands::VerifyCheck(args) => utils::block_on(args.run()),
        Subcommands::Cache(cmd) => match cmd.sub {
            CacheSubcommands::Clean(cmd) => cmd.run(),
            CacheSubcommands::Ls(cmd) => cmd.run(),
        },
        Subcommands::Create(cmd) => utils::block_on(cmd.run()),
        Subcommands::Update(cmd) => cmd.run(),
        Subcommands::Install(cmd) => cmd.run(),
        Subcommands::Remove(cmd) => cmd.run(),
        Subcommands::Remappings(cmd) => cmd.run(),
        Subcommands::Init(cmd) => cmd.run(),
        Subcommands::Completions { shell } => {
            generate(shell, &mut Opts::command(), "forge", &mut std::io::stdout());
            Ok(())
        }
        Subcommands::GenerateFigSpec => {
            clap_complete::generate(
                clap_complete_fig::Fig,
                &mut Opts::command(),
                "forge",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Subcommands::Clean { root } => {
            let config = utils::load_config_with_root(root);
            config.project()?.cleanup()?;
            Ok(())
        }
        Subcommands::Snapshot(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_snapshot(cmd))
            } else {
                utils::block_on(cmd.run())
            }
        }
        Subcommands::Fmt(cmd) => cmd.run(),
        Subcommands::Config(cmd) => cmd.run(),
        Subcommands::Flatten(cmd) => cmd.run(),
        Subcommands::Inspect(cmd) => cmd.run(),
        Subcommands::UploadSelectors(args) => utils::block_on(args.run()),
        Subcommands::Tree(cmd) => cmd.run(),
        Subcommands::Geiger(cmd) => {
            let check = cmd.check;
            let n = cmd.run()?;
            if check && n > 0 {
                std::process::exit(n as i32);
            }
            Ok(())
        }
        Subcommands::Doc(cmd) => cmd.run(),
        Subcommands::Selectors { command } => utils::block_on(command.run()),
        Subcommands::Generate(cmd) => match cmd.sub {
            GenerateSubcommands::Test(cmd) => cmd.run(),
        },
    }
}
