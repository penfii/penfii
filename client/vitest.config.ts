import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// Screen tests (.test.tsx) need a DOM; model tests (.test.ts) run in plain Node, which is much cheaper
// than building a jsdom window per file. A .test.ts that needs the DOM (an editor) says so with
// `// @vitest-environment jsdom` at its top.
export default defineConfig({
  plugins: [
    react({
      exclude: /node_modules\/(?!@penfii\/)/,
    }),
  ],
  resolve: {
    dedupe: ['react', 'react-dom'],
  },
  test: {
    setupFiles: ['packages/client/src/test/setup.ts'],
    projects: [
      {
        extends: true,
        test: { name: 'node', environment: 'node', include: ['packages/**/*.test.ts'] },
      },
      {
        extends: true,
        test: {
          name: 'dom',
          environment: 'jsdom',
          include: ['packages/**/*.test.tsx'],
          setupFiles: ['packages/client/src/test/setupDom.ts'],
        },
      },
    ],
  },
})
