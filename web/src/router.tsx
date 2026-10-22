import { createRootRoute, createRoute, createRouter, lazyRouteComponent } from '@tanstack/react-router'
import { Layout } from './components/Layout'
import * as routes from './components/Routes'
import { basePath } from './served'

const root = createRootRoute({ component: Layout })

export const router = createRouter({
  basepath: basePath(),
  routeTree: root.addChildren([
    createRoute({ getParentRoute: () => root, path: '/', component: routes.HomeRoute }),
    createRoute({
      getParentRoute: () => root,
      path: '/new',
      component: lazyRouteComponent(() => import('./components/NewThread'), 'NewThread'),
      validateSearch: (search: Record<string, unknown>): { prompt?: string } =>
        typeof search.prompt === 'string' ? { prompt: search.prompt } : {},
    }),
    createRoute({ getParentRoute: () => root, path: '/projects', component: lazyRouteComponent(() => import('./components/Projects'), 'Projects') }),
    createRoute({ getParentRoute: () => root, path: '/projects/add', component: lazyRouteComponent(() => import('./components/Projects'), 'AddProjectPage') }),
    createRoute({ getParentRoute: () => root, path: '/orgs', component: lazyRouteComponent(() => import('./components/Orgs'), 'OrgsRoute') }),
    createRoute({ getParentRoute: () => root, path: '/devices', component: lazyRouteComponent(() => import('./components/Devices'), 'Devices') }),
    createRoute({ getParentRoute: () => root, path: '/needs-you', component: lazyRouteComponent(() => import('./components/NeedsYou'), 'NeedsYou') }),
    createRoute({ getParentRoute: () => root, path: '/graph', component: lazyRouteComponent(() => import('./components/Graph'), 'Graph') }),
    createRoute({
      getParentRoute: () => root,
      path: '/search',
      component: lazyRouteComponent(() => import('./components/Search'), 'Search'),
      validateSearch: (search: Record<string, unknown>): { q?: string } => (typeof search.q === 'string' ? { q: search.q } : {}),
    }),
    createRoute({ getParentRoute: () => root, path: '/t/$id', component: routes.ThreadRoute }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/files/$', component: lazyRouteComponent(() => import('./components/FilePage'), 'FileRoute') }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/live', component: lazyRouteComponent(() => import('./components/Live'), 'LivePage') }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/reconcile', component: lazyRouteComponent(() => import('./components/Reconcile'), 'Reconcile') }),
    createRoute({ getParentRoute: () => root, path: '/mrs', component: lazyRouteComponent(() => import('./components/MergeRequests'), 'MergeRequests') }),
    createRoute({ getParentRoute: () => root, path: '/mr/$id', component: lazyRouteComponent(() => import('./components/MergeRequests'), 'MergeRequestRoute') }),
    createRoute({ getParentRoute: () => root, path: '/inbox', component: lazyRouteComponent(() => import('./components/Inbox'), 'Inbox') }),
  ]),
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
