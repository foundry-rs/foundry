# @foundry-rs npm Packages

This folder contains the npm packages for the Foundry CLI.

- `@foundry-rs/forge`,
- `@foundry-rs/anvil` (soon),

## Local publish & Test

The npm folder contains the meta package `@foundry-rs/forge` and per-arch packages (e.g. `@foundry-rs/forge-darwin-arm64`). This guide shows how to publish them to a local registry and test via npx/bunx.

### Prerequisites

- Node.js LTS, npm, and Bun installed (`bun -v`).
- Docker (recommended) or a local [Verdaccio](https://verdaccio.org) install.
- Optional: Rust toolchain if you want to build a fresh `forge` binary (`cargo build --release -p forge`).

Start a local npm registry (Verdaccio) with Docker:

- Docker: `docker run -it --rm --name verdaccio -p 4873:4873 verdaccio/verdaccio`
- Verify: open `http://localhost:4873` in a browser
- Note: you might need to bump `max_body_size` in [Verdaccio `config.yaml`](https://verdaccio.org/docs/configuration#max-body-size):

  ```yaml
  max_body_size: 300mb # default is 10mb
  ```

  I enter the docker image then `echo 'max_body_size: 300mb' >> /verdaccio/conf/config.yaml`

Authenticate npm to Verdaccio

- Create a user/token: `npm adduser --registry http://localhost:4873 --scope=@foundry-rs`
- Ensure your auth token is present (either in `~/.npmrc` or a project `.npmrc`):
  - `registry=http://localhost:4873`
  - `//localhost:4873/:_authToken=YOUR_TOKEN`

#### Quick publish (macOS arm64)

- From `npm/` in this repo:
  - `export NPM_REGISTRY_URL=http://localhost:4873`
  - `export NPM_TOKEN=localtesttoken` # required by scripts; any non-empty value works
  - `bun install`
  - `./scripts/setup-local.sh`
- This publishes `@foundry-rs/forge-darwin-arm64` and then `@foundry-rs/forge` to your local registry.

### Manual publish (any platform)

- Build wrappers: `cd npm && bun install && npm run build`
- If you have a local `forge` binary, stage it for your platform:
  - Example (macOS arm64):
    - `cargo build --release -p forge`
    - `cd npm`
    - `ARCH=arm64 PLATFORM_NAME=darwin FORGE_BIN_PATH=../target/release/forge bun ./scripts/prepublish.ts`
- Publish the platform package, then the meta package (versions auto-synced from Cargo.toml unless overridden by `VERSION_NAME`):
  - `export NPM_REGISTRY_URL=http://localhost:4873`
  - `export NPM_TOKEN=localtesttoken`
  - `bun run ./scripts/publish.ts ./@foundry-rs/forge-<platform>-<arch>`
  - `bun run ./scripts/publish.ts ./@foundry-rs/forge`

#### Run from a test workspace

- Use the provided workspace: `cd npm/test/workspace`
- Registry config is already set (`.npmrc` and `bunfig.toml` point to `http://localhost:4873`).
- With npm: `npx @foundry-rs/forge --version`
- With Bun: `bunx @foundry-rs/forge --version`
- Alternatively from anywhere, force the local registry:
  - `npm_config_registry=http://localhost:4873 npx @foundry-rs/forge --version`
  - `REGISTRY_URL=http://localhost:4873 bunx @foundry-rs/forge --version`

#### Notes

- The meta package’s `postinstall` either installs the platform-specific optionalDependency or downloads its tarball from the configured registry.
- Publish arch packages first, then the meta package; the publish script auto-updates `optionalDependencies` to the same version.
- If `npm publish` returns 401, ensure you ran `npm adduser` against `http://localhost:4873` and that your token is present in `.npmrc` or provided via `NODE_AUTH_TOKEN`.
