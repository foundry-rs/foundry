# `forgeup`

Update or revert to a specific Foundry branch with ease.

## Installing

```sh
curl https://raw.githubusercontent.com/gakonst/foundry/master/forgeup/install | bash
```

## Usage

To install the **nightly** version:

```sh
forgeup
```

To install a specific **version** (in this case the `nightly` version):

```sh
forgeup --version nightly
```

To install a specific **branch** (in this case the `release/0.1.0` branch's latest commit):

```sh
forgeup --branch release/0.1.0
```

To install a **fork's main branch** (in this case `transmissions11/foundry`'s main branch):

```sh
forgeup --repo transmissions11/foundry
```

To install a **specific branch in a fork** (in this case the `patch-10` branch's latest commit in `transmissions11/foundry`):

```sh
forgeup --repo transmissions11/foundry --branch patch-10
```

---

**Tip**: All flags have a single character shorthand equivalent! You can use `-v` instead of `--version`, etc.

---
