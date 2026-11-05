import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [
    react({
      exclude: /node_modules\/(?!@penfii\/)/,
    }),
  ],
  resolve: {
    dedupe: ['react', 'react-dom'],
  },
  optimizeDeps: {
    exclude: ['@penfii/client', '@penfii/api-client', '@penfii/contracts'],
  },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/healthz': { target: 'http://127.0.0.1:8080', changeOrigin: true },
      '/readyz': { target: 'http://127.0.0.1:8080', changeOrigin: true },
      '/v1': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
        ws: true,
      },
      // MCP OAuth Authorization Server, discovery documents and the MCP endpoint (one origin).
      '/oauth': { target: 'http://127.0.0.1:8080', changeOrigin: true, xfwd: true },
      '/mcp': { target: 'http://127.0.0.1:8080', changeOrigin: true, xfwd: true },
      '/.well-known': { target: 'http://127.0.0.1:8080', changeOrigin: true, xfwd: true },
    },
  },
})
