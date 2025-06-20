interface ImportMetaEnv {
  readonly CI: string
  readonly NPM_TOKEN: string
  readonly BUN_AUTH_TOKEN: string

  readonly NODE_ENV: 'development' | 'production'

  readonly ARCH: string
  readonly PLATFORM_NAME: string
  readonly FORGE_BIN_PATH: string
  readonly FOUNDRY_OUT_DIR: string
  readonly TARGET: string
  readonly PROFILE: string
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
