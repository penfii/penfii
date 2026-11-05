import { QueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (failureCount, error) => {
        if (isApiError(error) && (error.status === 401 || error.status === 404 || error.status === 409)) return false
        return failureCount < 1
      },
    },
    mutations: { retry: false },
  },
})
