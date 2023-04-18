use clap::{CommandFactory, Parser};
use clap_complete::generate;
use foundry_cli::{
    cmd::{
        forge::{cache::CacheSubcommands, watch},
        Cmd,
    },
    handler,
    opts::forge::{Opts, Subcommands},
    utils,
};

fn main() -> eyre::Result<()> {
    utils::load_dotenv();
    handler::install()?;
    utils::subscriber();
    utils::enable_paint();

    let opts = Opts::parse();
    match opts.sub {
        Subcommands::Test(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_test(cmd))?;
            } else {
                let outcome = cmd.run()?;
                outcome.ensure_ok()?;
            }
        }
        Subcommands::Script(cmd) => {
            // install the shell before executing the command
            foundry_common::shell::set_shell(foundry_common::shell::Shell::from_args(
                cmd.opts.args.silent,
                cmd.json,
            ))?;
            utils::block_on(cmd.run_script())?;
        }
        Subcommands::Coverage(cmd) => {
            cmd.run()?;
        }
        Subcommands::Bind(cmd) => {
            cmd.run()?;
        }
        Subcommands::Build(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_build(cmd))?;
            } else {
                cmd.run()?;
            }
        }
        Subcommands::Debug(cmd) => {
            utils::block_on(cmd.debug())?;
        }
        Subcommands::VerifyContract(args) => {
            utils::block_on(args.run())?;
        }
        Subcommands::VerifyCheck(args) => {
            utils::block_on(args.run())?;
        }
        Subcommands::Cache(cmd) => match cmd.sub {
            CacheSubcommands::Clean(cmd) => {
                cmd.run()?;
            }
            CacheSubcommands::Ls(cmd) => {
                cmd.run()?;
            }
        },
        Subcommands::Create(cmd) => {
            utils::block_on(cmd.run())?;
        }
        Subcommands::Update(cmd) => cmd.run()?,
        Subcommands::Install(cmd) => {
            cmd.run()?;
        }
        Subcommands::Remove(cmd) => {
            cmd.run()?;
        }
        Subcommands::Remappings(cmd) => {
            cmd.run()?;
        }
        Subcommands::Init(cmd) => {
            cmd.run()?;
        }
        Subcommands::Completions { shell } => {
            generate(shell, &mut Opts::command(), "forge", &mut std::io::stdout())
        }
        Subcommands::GenerateFigSpec => clap_complete::generate(
            clap_complete_fig::Fig,
            &mut Opts::command(),
            "forge",
            &mut std::io::stdout(),
        ),
        Subcommands::Clean { root } => {
            let config = utils::load_config_with_root(root);
            config.project()?.cleanup()?;
        }
        Subcommands::Snapshot(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_snapshot(cmd))?;
            } else {
                cmd.run()?;
            }
        }
        Subcommands::Fmt(cmd) => {
            cmd.run()?;
        }
        Subcommands::Config(cmd) => {
            cmd.run()?;
        }
        Subcommands::Flatten(cmd) => {
            cmd.run()?;
        }
        Subcommands::Inspect(cmd) => {
            cmd.run()?;
        }
        Subcommands::UploadSelectors(args) => {
            utils::block_on(args.run())?;
        }
        Subcommands::Tree(cmd) => {
            cmd.run()?;
        }
        Subcommands::Geiger(cmd) => {
            let check = cmd.check;
            let n = cmd.run()?;
            if check && n > 0 {
                std::process::exit(n as i32);
            }
        }
        Subcommands::Doc(cmd) => {
            cmd.run()?;
        }
    }

    Ok(())
}
