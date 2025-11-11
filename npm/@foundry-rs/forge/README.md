# [Forge](https://getfoundry.sh/forge)

Forge is a command-line tool that ships with Foundry. Forge tests, builds, and deploys your smart contracts.
The forge binary can be used both within and outside of a Foundry project.

## Usage

### One-off commands

Example

```sh
npx --yes @foundry-rs/forge@nightly init
```

More generally

```sh
npx --yes @foundry-rs/forge@<version|nightly> <command> [args...]
```

### Install then use

locally to your project

```sh
npm add @foundry-rs/forge@nightly
npx forge <command> [args...]
```

globally

```sh
npm add --global @foundry-rs/forge@nightly
forge <command> [args...]
```

---

Also works with `deno`, `bun`, and `pnpm`:

```sh
deno run --quiet --allow-all npm:@foundry-rs/forge@nightly <command> [args...]
```

```sh
bun x @foundry-rs/forge@nightly <command> [args...]
```

```sh
pnpm dlx --silent @foundry-rs/forge@nightly <command> [args...]
```
