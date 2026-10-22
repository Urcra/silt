// Each route's view in the shell's page; a view drawn as its board takes
// its own `Page`, the rest a plain one until they are (ADR 0035).

import { useParams } from '@tanstack/react-router'
import type { ComponentType } from 'react'
import { Devices } from './Devices'
import { FilePage } from './FilePage'
import { Graph } from './Graph'
import { Inbox } from './Inbox'
import { LivePage } from './Live'
import { MergeRequest, MergeRequests } from './MergeRequests'
import { NeedsYou } from './NeedsYou'
import { NewThread } from './NewThread'
import { Orgs } from './Orgs'
import { Page } from './Page'
import { AddProjectPage, Projects } from './Projects'
import { Reconcile } from './Reconcile'
import { Search } from './Search'
import { ThreadList } from './ThreadList'
import { ThreadView } from './ThreadView'

function Plain({ view: View }: { view: ComponentType }) {
  return (
    <Page width="wide">
      <View />
    </Page>
  )
}

export const HomeRoute = ThreadList
export const NewThreadRoute = NewThread
export const ProjectsRoute = Projects
export const AddProjectRoute = AddProjectPage
export const OrgsRoute = () => <Plain view={Orgs} />
export const DevicesRoute = Devices
export const NeedsYouRoute = NeedsYou
export const GraphRoute = Graph
export const SearchRoute = Search
export const FileRoute = () => <Plain view={FilePage} />
export const MergeRequestsRoute = MergeRequests
export const InboxRoute = Inbox

export function ThreadRoute() {
  const { id } = useParams({ from: '/t/$id' })
  return <ThreadView key={id} id={id} />
}

export const LiveRoute = LivePage
export const ReconcileRoute = Reconcile

export function MergeRequestRoute() {
  const { id } = useParams({ from: '/mr/$id' })
  return <MergeRequest key={id} id={id} />
}
