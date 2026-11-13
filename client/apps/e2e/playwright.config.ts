import { defineConfig, devices } from '@playwright/test'

// End-to-end tests run against a running Compose stack (docker/), a real browser and the real server.
// They are not part of `make check`; see docs/development.md ("End-to-end tests") for how and why.
export default defineConfig({
  testDir: './tests',
  // One worker: the tests share one Workspace and a person is signed in once per browser context, but the
  // server rate-limits logins, so runs are kept sequential and each test makes its own data.
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [['list']],
  use: {
    baseURL: process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080',
    locale: 'en-US',
    // The app registers a service worker for Web Push; WebKit then serves some requests around `page.route`, so tests that
    // fake an answer would not see their route used. Nothing under test needs the worker.
    serviceWorkers: 'block',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  // Chromium is the default. `E2E_BROWSERS=chromium,firefox,webkit` adds the others (`npx playwright install firefox webkit`;
  // WebKit also needs the system libraries `npx playwright install-deps webkit` installs).
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
  ].filter((project) => (process.env.E2E_BROWSERS ?? 'chromium').split(',').includes(project.name)),
})
