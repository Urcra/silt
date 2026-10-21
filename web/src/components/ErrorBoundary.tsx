import { Component, type ReactNode } from 'react'
import { Banner, Button } from '../ui'

/** Keeps one panel's failure from taking the whole app down. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null }

  static getDerivedStateFromError(error: Error) {
    return { error }
  }

  render() {
    if (!this.state.error) return this.props.children
    return (
      <div className="p-4">
        <Banner tone="danger" urgent>
          <p>This part of the page failed: {this.state.error.message}</p>
          <Button className="mt-2" onClick={() => this.setState({ error: null })}>
            Try again
          </Button>
        </Banner>
      </div>
    )
  }
}
