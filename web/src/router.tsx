import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
import { Devices } from './components/Devices'
import { FilePage } from './components/FilePage'
import { Graph } from './components/Graph'
import { Inbox } from './components/Inbox'
import { Layout, MergeRequestPage, ThreadPage } from './components/Layout'
import { MergeRequests } from './components/MergeRequests'
import { NeedsYou } from './components/NeedsYou'
import { NewThread } from './components/NewThread'
import { Orgs } from './components/Orgs'
import { Projects } from './components/Projects'
import { ThreadList } from './components/ThreadList'
import { basePath } from './served'

const root = createRootRoute({ component: Layout })

export const router = createRouter({
  basepath: basePath(),
  routeTree: root.addChildren([
    createRoute({ getParentRoute: () => root, path: '/', component: ThreadList }),
    createRoute({ getParentRoute: () => root, path: '/new', component: NewThread }),
    createRoute({ getParentRoute: () => root, path: '/projects', component: Projects }),
    createRoute({ getParentRoute: () => root, path: '/orgs', component: Orgs }),
    createRoute({ getParentRoute: () => root, path: '/devices', component: Devices }),
    createRoute({ getParentRoute: () => root, path: '/needs-you', component: NeedsYou }),
    createRoute({ getParentRoute: () => root, path: '/graph', component: Graph }),
    createRoute({ getParentRoute: () => root, path: '/t/$id', component: ThreadPage }),
    createRoute({ getParentRoute: () => root, path: '/t/$id/files/$', component: FilePage }),
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
