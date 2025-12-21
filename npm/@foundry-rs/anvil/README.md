# [Anvil](https://getfoundry.sh/anvil)

Anvil is a fast local Ethereum development node.
The anvil binary can be used both within and outside of a Foundry project.

## Usage

### One-off commands

Example

```sh
npx --yes @foundry-rs/anvil@nightly
```

More generally

```sh
npx --yes @foundry-rs/anvil@<version|nightly> [args...]
```

### Install then use

locally to your project

```sh
npm add @foundry-rs/anvil@nightly
npx anvil [args...]
```

globally

```sh
npm add --global @foundry-rs/anvil@nightly
anvil [args...]
```

---

Also works with `deno`, `bun`, and `pnpm`:

```sh
deno run --quiet --allow-all npm:@foundry-rs/anvil@nightly [args...]
```

```sh
bun x @foundry-rs/anvil@nightly [args...]
```

```sh
pnpm dlx --silent @foundry-rs/anvil@nightly [args...]
```
