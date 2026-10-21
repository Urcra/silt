import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
import { Inbox } from './components/Inbox'
import { Layout, MergeRequestPage, ThreadPage } from './components/Layout'
import { MergeRequests } from './components/MergeRequests'
import { NewThread } from './components/NewThread'
import { Orgs } from './components/Orgs'
import { Projects } from './components/Projects'
import { ThreadList } from './components/ThreadList'

const root = createRootRoute({ component: Layout })

export const router = createRouter({
  routeTree: root.addChildren([
    createRoute({ getParentRoute: () => root, path: '/', component: ThreadList }),
    createRoute({ getParentRoute: () => root, path: '/new', component: NewThread }),
    createRoute({ getParentRoute: () => root, path: '/projects', component: Projects }),
    createRoute({ getParentRoute: () => root, path: '/orgs', component: Orgs }),
    createRoute({ getParentRoute: () => root, path: '/t/$id', component: ThreadPage }),
    createRoute({ getParentRoute: () => root, path: '/mrs', component: MergeRequests }),
    createRoute({ getParentRoute: () => root, path: '/mr/$id', component: MergeRequestPage }),
    createRoute({ getParentRoute: () => root, path: '/inbox', component: Inbox }),
  ]),
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
