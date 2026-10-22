import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
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
      component: routes.NewThreadRoute,
      validateSearch: (search: Record<string, unknown>): { prompt?: string } =>
        typeof search.prompt === 'string' ? { prompt: search.prompt } : {},
    }),
    createRoute({ getParentRoute: () => root, path: '/projects', component: routes.ProjectsRoute }),
    createRoute({ getParentRoute: () => root, path: '/projects/add', component: routes.AddProjectRoute }),
    createRoute({ getParentRoute: () => root, path: '/orgs', component: routes.OrgsRoute }),
    createRoute({ getParentRoute: () => root, path: '/devices', component: routes.DevicesRoute }),
    createRoute({ getParentRoute: () => root, path: '/needs-you', component: routes.NeedsYouRoute }),
    createRoute({ getParentRoute: () => root, path: '/graph', component: routes.GraphRoute }),
    createRoute({
      getParentRoute: () => root,
      path: '/search',
      component: routes.SearchRoute,
      validateSearch: (search: Record<string, unknown>): { q?: string } => (typeof search.q === 'string' ? { q: search.q } : {}),
    }),
    createRoute({ getParentRoute: () => root, path: '/t/$id', component: routes.ThreadRoute }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/files/$', component: routes.FileRoute }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/live', component: routes.LiveRoute }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/reconcile', component: routes.ReconcileRoute }),
    createRoute({ getParentRoute: () => root, path: '/mrs', component: routes.MergeRequestsRoute }),
    createRoute({ getParentRoute: () => root, path: '/mr/$id', component: routes.MergeRequestRoute }),
    createRoute({ getParentRoute: () => root, path: '/inbox', component: routes.InboxRoute }),
  ]),
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
