# [Chisel](https://getfoundry.sh/chisel)

Chisel is a fast, utilitarian, and verbose Solidity REPL.
The chisel binary can be used both within and outside of a Foundry project.

## Usage

### One-off commands

Example

```sh
npx --yes @foundry-rs/chisel@nightly
```

More generally

```sh
npx --yes @foundry-rs/chisel@<version|nightly> [args...]
```

### Install then use

locally to your project

```sh
npm add @foundry-rs/chisel@nightly
npx chisel [args...]
```

globally

```sh
npm add --global @foundry-rs/chisel@nightly
chisel [args...]
```

---

Also works with `deno`, `bun`, and `pnpm`:

```sh
deno run --quiet --allow-all npm:@foundry-rs/chisel@nightly [args...]
```

```sh
bun x @foundry-rs/chisel@nightly [args...]
```

```sh
pnpm dlx --silent @foundry-rs/chisel@nightly [args...]
```
