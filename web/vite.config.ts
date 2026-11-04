import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5175, // Use 5175 to avoid port conflicts
    // The repo lives on /mnt/c under WSL, where file changes raise no
    // inotify events; without polling Vite keeps serving stale modules.
    watch: { usePolling: true, interval: 500 },
    proxy: {
      '/api': {
        target: 'http://localhost:3000',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://localhost:3000',
        ws: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
