import { useQuery } from '@tanstack/react-query'
import { queries } from '../../queries'

export function useSession() {
  return useQuery(queries.me())
}
