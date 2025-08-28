import { defineConfig } from 'vite'

export default defineConfig({
  root: '.',
  server: {
    open: true,
  },
  assetsInclude: ['**/*.wasm'],
  build: {
    target: 'esnext',
    emptyOutDir: true,
    assetsDir: 'build',
  },
})
