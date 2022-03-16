# `foundryup`

Update or revert to a specific Foundry branch with ease.

## Installing

```sh
curl -L https://foundry.paradigm.xyz | bash
```

## Usage

To install the **nightly** version:

```sh
foundryup
```

To install a specific **version** (in this case the `nightly` version):

```sh
foundryup --version nightly
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

To install a local directory or repository (e.g. one located at `~/git/foundry`, assuming you're in the home directory)
##### Note: --branch, --repo, and --version flags are ignored during local installations. 

```sh
foundryup --path ./git/foundry
```

---

**Tip**: All flags have a single character shorthand equivalent! You can use `-v` instead of `--version`, etc.

---
