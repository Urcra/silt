import { createRootRoute, createRoute, createRouter, lazyRouteComponent, redirect } from '@tanstack/react-router'
import { Layout } from './components/Layout'
import * as routes from './components/Routes'
import { basePath, hosted } from './served'
import { phone } from './shell'
import { designSearch } from './designSearch'

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
    createRoute({
      getParentRoute: () => root,
      path: '/settings/$section',
      component: lazyRouteComponent(() => import('./components/Settings'), 'Settings'),
    }),
    // One design system's page (ADR 0088); the hosted app shows none yet.
    createRoute({
      getParentRoute: () => root,
      path: '/settings/design-systems/system',
      component: lazyRouteComponent(() => import('./components/DesignSystems'), 'DesignSystemPage'),
      validateSearch: designSearch,
      beforeLoad: () => {
        if (hosted()) throw redirect({ to: '/settings/$section', params: { section: 'projects' }, replace: true })
      },
    }),
    // One plugin's page (ADR 0104); plugins are this machine's.
    createRoute({
      getParentRoute: () => root,
      path: '/settings/plugins/plugin',
      component: lazyRouteComponent(() => import('./components/Plugins'), 'PluginPage'),
      validateSearch: (search: Record<string, unknown>) => ({ id: typeof search.id === 'string' ? search.id : '' }),
      beforeLoad: () => {
        if (hosted()) throw redirect({ to: '/settings/$section', params: { section: 'projects' }, replace: true })
      },
    }),
    // One theme's page (ADR 0098).
    createRoute({
      getParentRoute: () => root,
      path: '/settings/appearance/theme',
      component: lazyRouteComponent(() => import('./components/Themes'), 'ThemePage'),
      validateSearch: (search: Record<string, unknown>) => ({ id: typeof search.id === 'string' ? search.id : '' }),
    }),
    moved('/projects', 'projects'),
    moved('/projects/add', 'add'),
    moved('/orgs', 'orgs'),
    moved('/devices', 'devices'),
    createRoute({
      getParentRoute: () => root,
      path: '/needs-you',
      component: lazyRouteComponent(() => import('./components/NeedsYou'), 'NeedsYou'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/graph',
      component: lazyRouteComponent(() => import('./components/Graph'), 'Graph'),
      validateSearch: (search: Record<string, unknown>): { thread?: string } =>
        typeof search.thread === 'string' ? { thread: search.thread } : {},
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/search',
      component: lazyRouteComponent(() => import('./components/Search'), 'Search'),
      validateSearch: (search: Record<string, unknown>): { q?: string } => (typeof search.q === 'string' ? { q: search.q } : {}),
    }),
    createRoute({ getParentRoute: () => root, path: '/t/$id', component: routes.ThreadRoute }),
    createRoute({
      getParentRoute: () => root,
      path: '/artifacts',
      component: lazyRouteComponent(() => import('./components/Gallery'), 'Gallery'),
    }),
    // A permalink to an artifact opens its thread with it open, at a version when one is named (ADR 0075).
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/artifacts/$artifact',
      beforeLoad: ({ params }) => {
        throw redirect({ to: '/t/$id', params: { id: params.id }, hash: `artifact-${params.artifact}`, replace: true })
      },
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/artifacts/$artifact/v/$n',
      beforeLoad: ({ params }) => {
        throw redirect({
          to: '/t/$id',
          params: { id: params.id },
          hash: `artifact-${params.artifact}-v${params.n}`,
          replace: true,
        })
      },
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/review',
      component: lazyRouteComponent(() => import('./components/Review'), 'ReviewRoute'),
      // A server has no thread's diff, and a phone reviews in the panel's sheet.
      beforeLoad: ({ params }) => {
        if (hosted() || phone()) throw redirect({ to: '/t/$id', params, replace: true })
      },
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/files/$',
      component: lazyRouteComponent(() => import('./components/FilePage'), 'FileRoute'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/live',
      component: lazyRouteComponent(() => import('./components/Live'), 'LivePage'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/t/$id/reconcile',
      component: lazyRouteComponent(() => import('./components/Reconcile'), 'Reconcile'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/mrs',
      component: lazyRouteComponent(() => import('./components/MergeRequests'), 'MergeRequests'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/mr/$id',
      component: lazyRouteComponent(() => import('./components/MergeRequests'), 'MergeRequestRoute'),
    }),
    createRoute({
      getParentRoute: () => root,
      path: '/inbox',
      component: lazyRouteComponent(() => import('./components/Inbox'), 'Inbox'),
    }),
  ]),
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
