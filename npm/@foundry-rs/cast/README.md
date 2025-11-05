# [Cast](https://getfoundry.sh/cast)

Cast is a Swiss Army knife for interacting with Ethereum applications from the command line.
You can make smart contract calls, send transactions, or retrieve any type of chain data - all from your command-line!
The cast binary can be used both within and outside of a Foundry project.

## Usage

### One-off commands

Example

```sh
npx --yes @foundry-rs/cast@nightly block-number
```

More generally

```sh
npx --yes @foundry-rs/cast@<version|nightly> <command> [args...]
```

### Install then use

locally to your project

```sh
npm add @foundry-rs/cast@nightly
npx cast <command> [args...]
```

globally

```sh
npm add --global @foundry-rs/cast@nightly
cast <command> [args...]
```

---

Also works with `deno`, `bun`, and `pnpm`:

```sh
deno run --quiet --allow-all npm:@foundry-rs/cast@nightly <command> [args...]
```

```sh
bun x @foundry-rs/cast@nightly <command> [args...]
```

```sh
pnpm dlx --silent @foundry-rs/cast@nightly <command> [args...]
```
