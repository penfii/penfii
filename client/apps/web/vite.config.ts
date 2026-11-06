import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const API = 'http://127.0.0.1:8080'

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
        target: API,
        changeOrigin: true,
        ws: true,
        // The browser's Origin is the dev server's (:5173), which the API refuses for a WebSocket unless it
        // is listed in PENFII_ALLOWED_ORIGINS (the Compose stack lists only its own public URL). Behind the
        // proxy the socket is the API's own, so it is sent with the API's origin.
        configure: (proxy) => {
          proxy.on('proxyReqWs', (request) => request.setHeader('origin', API))
        },
      },
      // MCP OAuth Authorization Server, discovery documents and the MCP endpoint (one origin).
      '/oauth': { target: 'http://127.0.0.1:8080', changeOrigin: true, xfwd: true },
      '/mcp': { target: 'http://127.0.0.1:8080', changeOrigin: true, xfwd: true },
      '/.well-known': { target: 'http://127.0.0.1:8080', changeOrigin: true, xfwd: true },
    },
  },
})
