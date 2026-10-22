// Each route's view in the shell's page; a view drawn as its board takes
// its own `Page`, the rest a plain one until they are (ADR 0035).

import { useParams } from '@tanstack/react-router'
import type { ComponentType } from 'react'
import { Devices } from './Devices'
import { FilePage } from './FilePage'
import { Graph } from './Graph'
import { Inbox } from './Inbox'
import { MergeRequest, MergeRequests } from './MergeRequests'
import { NeedsYou } from './NeedsYou'
import { NewThread } from './NewThread'
import { Orgs } from './Orgs'
import { Page } from './Page'
import { Projects } from './Projects'
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
export const NewThreadRoute = () => <Plain view={NewThread} />
export const ProjectsRoute = () => <Plain view={Projects} />
export const OrgsRoute = () => <Plain view={Orgs} />
export const DevicesRoute = () => <Plain view={Devices} />
export const NeedsYouRoute = () => <Plain view={NeedsYou} />
export const GraphRoute = () => <Plain view={Graph} />
export const SearchRoute = () => <Plain view={Search} />
export const FileRoute = () => <Plain view={FilePage} />
export const MergeRequestsRoute = () => <Plain view={MergeRequests} />
export const InboxRoute = () => <Plain view={Inbox} />

export function ThreadRoute() {
  const { id } = useParams({ from: '/t/$id' })
  return (
    <Page width="wide">
      <ThreadView id={id} />
    </Page>
  )
}

export function MergeRequestRoute() {
  const { id } = useParams({ from: '/mr/$id' })
  return (
    <Page width="wide">
      <MergeRequest id={id} />
    </Page>
  )
}
