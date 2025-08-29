import tailwindcss from '@tailwindcss/vite'
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
    plugins: [tailwindcss()],
    build: {
      target: 'esnext',
      emptyOutDir: true,
      rollupOptions: {
        output: {
          manualChunks(id) {
            if (!id.includes('node_modules')) return
            const [, modulePath] = id.split('node_modules/')
            const [topLevelFolder] = modulePath?.split('/')
            if (topLevelFolder !== '.deno') return topLevelFolder

            // changed . to ?. for the two lines below:
            const [, scopedPackageName] = modulePath?.split('/')
            return scopedPackageName?.split(
              '@',
            )[scopedPackageName.startsWith('@') ? 1 : 0]
          },
        },
      },
    },
  }
})
