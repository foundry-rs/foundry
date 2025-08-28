import { defineConfig, loadEnv } from 'vite'

export default defineConfig((config) => {
  const env = loadEnv(config.mode, Deno.cwd(), '')
  const allowedHosts = env.VITE_ALLOWED_HOSTS?.split(',') ?? []
  return {
    root: '.',
    server: {
      open: true,
      allowedHosts,
    },
    assetsInclude: ['**/*.wasm'],
    build: {
      target: 'esnext',
      emptyOutDir: true,
      // assetsDir: 'build',
    },
  }
})
