import { createRootRoute, createRoute, createRouter, lazyRouteComponent } from '@tanstack/react-router'
import { Layout } from './components/Layout'
import { ThreadList } from './components/ThreadList'
import { ThreadPage } from './components/ThreadView'
import { basePath } from './served'

const root = createRootRoute({ component: Layout })

// The pages people open most, the threads and one thread, come with the
// first screen; each other page comes in its own chunk when it's opened.
export const router = createRouter({
  basepath: basePath(),
  routeTree: root.addChildren([
    createRoute({ getParentRoute: () => root, path: '/', component: ThreadList }),
    createRoute({
      getParentRoute: () => root,
      path: '/new',
      component: lazyRouteComponent(() => import('./components/NewThread'), 'NewThread'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/projects',
      component: lazyRouteComponent(() => import('./components/Projects'), 'Projects'),
    }),
    createRoute({ getParentRoute: () => root, path: '/orgs', component: lazyRouteComponent(() => import('./components/Orgs'), 'Orgs') }),
    createRoute({
      getParentRoute: () => root,
      path: '/devices',
      component: lazyRouteComponent(() => import('./components/Devices'), 'Devices'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/needs-you',
      component: lazyRouteComponent(() => import('./components/NeedsYou'), 'NeedsYou'),
    }),
    createRoute({ getParentRoute: () => root, path: '/graph', component: lazyRouteComponent(() => import('./components/Graph'), 'Graph') }),
    createRoute({ getParentRoute: () => root, path: '/t/$id', component: ThreadPage }),
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/files/$',
      component: lazyRouteComponent(() => import('./components/FilePage'), 'FilePage'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/mrs',
      component: lazyRouteComponent(() => import('./components/MergeRequests'), 'MergeRequests'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/mr/$id',
      component: lazyRouteComponent(() => import('./components/MergeRequests'), 'MergeRequestPage'),
    }),
    createRoute({ getParentRoute: () => root, path: '/inbox', component: lazyRouteComponent(() => import('./components/Inbox'), 'Inbox') }),
  ]),
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
