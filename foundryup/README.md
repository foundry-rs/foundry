# `foundryup-polkadot`

Update or revert to a specific Foundry branch with ease.

`foundryup-polkadot` supports installing and managing multiple versions.

## Installing

```sh
curl -L https://raw.githubusercontent.com/paritytech/foundry-polkadot/refs/heads/master/foundryup/foundryup | bash
```

## Usage

To install the **nightly** version:

```sh
foundryup-polkadot
```

To **install** a specific **version** (in this case the `nightly` version):

```sh
foundryup-polkadot --install nightly
```

To **list** all **versions** installed:

```sh
foundryup-polkadot --list
```

To switch between different versions and **use**:

```sh
foundryup-polkadot --use nightly-00efa0d5965269149f374ba142fb1c3c7edd6c94
```

To install a specific **branch** (in this case the `release/0.1.0` branch's latest commit):

```sh
foundryup-polkadot --branch release/0.1.0
```

To install a **fork's main branch** (in this case `transmissions11/foundry`'s main branch):

```sh
foundryup-polkadot --repo transmissions11/foundry
```

To install a **specific branch in a fork** (in this case the `patch-10` branch's latest commit in `transmissions11/foundry`):

```sh
foundryup-polkadot --repo transmissions11/foundry --branch patch-10
```

To install from a **specific Pull Request**:

```sh
foundryup-polkadot --pr 1071
```

To install from a **specific commit**:

```sh
foundryup-polkadot -C 94bfdb2
```

To install a local directory or repository (e.g. one located at `~/git/foundry`, assuming you're in the home directory)

#### Note: --branch, --repo, and --version flags are ignored during local installations.

```sh
foundryup-polkadot --path ./git/foundry
```

---

**Tip**: All flags have a single character shorthand equivalent! You can use `-i` instead of `--install`, etc.

---

## Uninstalling

Foundry contains everything in a `.foundry` directory, usually located in `/home/<user>/.foundry/` on Linux, `/Users/<user>/.foundry/` on MacOS and `C:\Users\<user>\.foundry` on Windows where `<user>` is your username.

To uninstall Foundry remove the `.foundry` directory.

#### Warning ⚠️: .foundry directory can contain keystores. Make sure to backup any keystores you want to keep.

Remove Foundry from PATH:

- Optionally Foundry can be removed from editing shell configuration file (`.bashrc`, `.zshrc`, etc.). To do so remove the line that adds Foundry to PATH:

```sh
export PATH="$PATH:/home/user/.foundry/bin"
```
