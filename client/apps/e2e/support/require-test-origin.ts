function requireLoopbackOrigin(value: string, variable: string) {
  const origin = new URL(value)
  if (!['http:', 'https:'].includes(origin.protocol) ||
      !['localhost', '127.0.0.1', '[::1]'].includes(origin.hostname) ||
      origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) {
    throw new Error(`${variable} must be a loopback HTTP(S) origin`)
  }
}

export default function requireTestOrigin() {
  const { E2E_BASE_URL, E2E_MAILPIT, E2E_RUNNER } = process.env
  if (!E2E_BASE_URL) {
    throw new Error('Run make e2e for a disposable stack or make e2e-external with an explicit test origin')
  }
  requireLoopbackOrigin(E2E_BASE_URL, 'E2E_BASE_URL')
  // In Docker the browser shares the disposable web container's network.
  if (E2E_MAILPIT && !(E2E_RUNNER === 'docker' && E2E_MAILPIT === 'http://mailpit:8025')) {
    requireLoopbackOrigin(E2E_MAILPIT, 'E2E_MAILPIT')
  }
}
