import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig(({ mode }) => {
  // Backend the dev server forwards /api and /ws to. The app itself always
  // calls same-origin paths (see src/shared/api.ts).
  const backend = loadEnv(mode, '.', '').VITE_BACKEND_URL || 'http://localhost:8000'

  return {
    plugins: [react()],
    server: {
      port: 5175, // Use 5175 to avoid port conflicts
      // The repo lives on /mnt/c under WSL, where file changes raise no
      // inotify events; without polling Vite keeps serving stale modules.
      watch: { usePolling: true, interval: 500 },
      proxy: {
        '/api': {
          target: backend,
          changeOrigin: true,
        },
        '/ws': {
          target: backend.replace(/^http/, 'ws'),
          ws: true,
        },
      },
    },
    build: {
      outDir: 'dist',
      emptyOutDir: true,
    },
  }
})
