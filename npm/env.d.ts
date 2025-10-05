interface ImportMetaEnv {
  readonly CI: string
  readonly NPM_TOKEN: string
  readonly BUN_AUTH_TOKEN: string

  readonly NODE_ENV: 'development' | 'production'

  readonly PROVENANCE: 'true' | 'false'

  // release.yml#jobs:release:strategy:matrix:include:-|target
  readonly TARGET:
    | 'x86_64-unknown-linux-gnu'
    | 'x86_64-unknown-linux-musl'
    | 'aarch64-unknown-linux-gnu'
    | 'aarch64-unknown-linux-musl'
    | 'x86_64-apple-darwin'
    | 'aarch64-apple-darwin'
    | 'x86_64-pc-windows-msvc'
  // <release.yml#jobs:release:strategy:matrix:include:-|arch>
  readonly ARCH: 'amd64' | 'arm64'
  readonly IS_NIGHTLY: 'true' | 'false'
  // `${(env.IS_NIGHTLY == 'true' && 'nightly') || needs.prepare.outputs.tag_name}`
  readonly VERSION_NAME: string
  // release.yml#jobs:release:strategy:matrix:include:-|platform
  readonly PLATFORM_NAME: 'linux' | 'alpine' | 'darwin' | 'win32'
  // `debug` / `release` / `maxperf` # <- always `maxperf`
  readonly PROFILE: 'debug' | 'release' | 'maxperf'

  // Used for local testing/development only
  readonly REGISTRY_URL: string
  readonly PACKAGE_PATH: string
  readonly ALLOW_NO_INTEGRITY: 'true' | 'false'
}

declare namespace NodeJS {
  interface ProcessEnv extends ImportMetaEnv {}
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}

declare namespace Bun {
  interface Env extends ImportMetaEnv {}
}
