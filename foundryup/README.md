# `foundryup`

Update or revert to a specific Foundry branch with ease.

`foundryup` supports installing and managing multiple versions.

## Installing

```sh
curl -L https://foundry.paradigm.xyz | bash
```

## Usage

To install the **nightly** version:

```sh
foundryup
```

To **install** a specific **version** (in this case the `nightly` version):

```sh
foundryup --install nightly
```

To **list** all **versions** installed:

```sh
foundryup --list
```

To switch between different versions and **use**:

```sh
foundryup --use nightly-00efa0d5965269149f374ba142fb1c3c7edd6c94
```

To install a specific **branch** (in this case the `release/0.1.0` branch's latest commit):

```sh
foundryup --branch release/0.1.0
```

To install a **fork's main branch** (in this case `transmissions11/foundry`'s main branch):

```sh
foundryup --repo transmissions11/foundry
```

To install a **specific branch in a fork** (in this case the `patch-10` branch's latest commit in `transmissions11/foundry`):

```sh
foundryup --repo transmissions11/foundry --branch patch-10
```

To install from a **specific Pull Request**:

```sh
foundryup --pr 1071
```

To install from a **specific commit**:

```sh
foundryup -C 94bfdb2
```

To install a local directory or repository (e.g. one located at `~/git/foundry`, assuming you're in the home directory)

##### Note: --branch, --repo, and --version flags are ignored during local installations.

```sh
foundryup --path ./git/foundry
```

---

**Tip**: All flags have a single character shorthand equivalent! You can use `-i` instead of `--install`, etc.

---


## Uninstalling

Foundry contains everything in a `.foundry` directory, usually located in `/home/user/.foundry/`.

- To uninstall Foundry remove the `.foundry` directory.

##### Note: .foundry directory can contain keystores. Make sure to backup any keystores you want to keep.


Remove Foundry from PATH:

- Optionally Foundry can be removed from editing shell configuration file (`.bashrc`, `.zshrc`, etc.) and remove the line that adds Foundry to PATH:

```
export PATH="$PATH:/home/user/.foundry/bin"
```

