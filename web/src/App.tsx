import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from '@tanstack/react-router'
import { useBusInvalidation } from './bus'
import { queryClient } from './query'
import { router } from './router'

function Listening() {
  useBusInvalidation()
  return null
}

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <Listening />
      <RouterProvider router={router} />
    </QueryClientProvider>
  )
}

export default App
