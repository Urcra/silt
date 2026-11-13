import { createRootRoute, createRoute, createRouter, lazyRouteComponent, redirect } from '@tanstack/react-router'
import { Layout } from './components/Layout'
import * as routes from './components/Routes'
import { basePath, hosted } from './served'
import { phone } from './shell'

const root = createRootRoute({ component: Layout })

/** What the new thread at `/` opens with: a project, or the picker open on a tab. */
export interface NewThreadSearch {
  project?: string
  pick?: 'projects' | 'new' | 'folder' | 'clone' | 'workspace'
}

const PICKS = ['projects', 'new', 'folder', 'clone', 'workspace']

function newThreadSearch(search: Record<string, unknown>): NewThreadSearch {
  return {
    ...(typeof search.project === 'string' ? { project: search.project } : {}),
    ...(typeof search.pick === 'string' && PICKS.includes(search.pick) ? { pick: search.pick as NewThreadSearch['pick'] } : {}),
  }
}

/** A path moved into Settings (ADR 0044). */
function moved(path: string, section: string) {
  return createRoute({
    getParentRoute: () => root,
    path,
    beforeLoad: () => {
      throw redirect({ to: '/settings/$section', params: { section }, replace: true })
    },
  })
}

export const router = createRouter({
  basepath: basePath(),
  routeTree: root.addChildren([
    createRoute({ getParentRoute: () => root, path: '/', component: routes.HomeRoute, validateSearch: newThreadSearch }),
    createRoute({
      getParentRoute: () => root,
      path: '/new',
      validateSearch: newThreadSearch,
      component: routes.NewRoute,
      beforeLoad: ({ search }) => {
        if (!phone() || hosted()) throw redirect({ to: '/', search, replace: true })
      },
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/chat',
      component: lazyRouteComponent(() => import('./components/NewChat'), 'NewChat'),
      // A server starts no agents.
      beforeLoad: () => {
        if (hosted()) throw redirect({ to: '/', replace: true })
      },
    }),
    createRoute({ getParentRoute: () => root, path: '/threads', component: routes.ThreadsRoute }),
    createRoute({
      getParentRoute: () => root,
      path: '/settings',
      beforeLoad: () => {
        throw redirect({ to: '/settings/$section', params: { section: 'projects' }, replace: true })
      },
    }),
    createRoute({ getParentRoute: () => root, path: '/settings/$section', component: lazyRouteComponent(() => import('./components/Settings'), 'Settings') }),
    moved('/projects', 'projects'),
    moved('/projects/add', 'add'),
    moved('/orgs', 'orgs'),
    moved('/devices', 'devices'),
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
