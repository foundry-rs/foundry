import { defineConfig } from 'vite'

export default defineConfig({
  root: '.',
  server: { open: true },
  assetsInclude: ['**/*.wasm'],
  build: { emptyOutDir: true, target: 'esnext' },
})
