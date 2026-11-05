import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import type { ReactNode } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { I18nProvider } from '../i18n/I18nProvider'

interface ProviderOptions {
  /** The URL the test starts at. Without it there is no router at all. */
  route?: string
  /** The route pattern `ui` is mounted at, so it reads its params from `route`. Needs `route`. */
  path?: string
  /** A query client the test keeps hold of; by default each call makes its own, so no cached answer leaks between tests. */
  client?: QueryClient
}

/**
 * Renders `ui` the way the app mounts it: a query client that does not retry, the i18n provider and,
 * when the test names a `route`, a memory router. For several routes at once, pass the `<Routes>`
 * as `ui` and a `route` to start from.
 */
export function renderWithProviders(ui: ReactNode, options: ProviderOptions = {}) {
  const client = options.client ?? new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const { route, path } = options
  const page = path === undefined ? ui : <Routes><Route path={path} element={ui} /></Routes>
  return {
    client,
    ...render(
      <QueryClientProvider client={client}>
        <I18nProvider>{route === undefined ? page : <MemoryRouter initialEntries={[route]}>{page}</MemoryRouter>}</I18nProvider>
      </QueryClientProvider>,
    ),
  }
}
