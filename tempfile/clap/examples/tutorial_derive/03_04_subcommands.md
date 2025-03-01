```console
$ 03_04_subcommands_derive help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_04_subcommands_derive[EXE] <COMMAND>

Commands:
  add   Adds files to myapp
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

$ 03_04_subcommands_derive help add
Adds files to myapp

Usage: 03_04_subcommands_derive[EXE] add [NAME]

Arguments:
  [NAME]  

Options:
  -h, --help     Print help
  -V, --version  Print version

$ 03_04_subcommands_derive add bob
'myapp add' was used, name is: Some("bob")

```

When specifying commands with `command: Commands`, they are required.
Alternatively, you could do `command: Option<Commands>` to make it optional.
```console
$ 03_04_subcommands_derive
? failed
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_04_subcommands_derive[EXE] <COMMAND>

Commands:
  add   Adds files to myapp
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

```

Since we specified [`#[command(propagate_version = true)]`][crate::Command::propagate_version],
the `--version` flag is available in all subcommands:
```console
$ 03_04_subcommands_derive --version
clap [..]

$ 03_04_subcommands_derive add --version
clap-add [..]

```
