import type { AgentCommand } from './api'

/** The command name being typed: the draft's first word when it starts with `/`, until a space ends it. */
export function typedCommand(draft: string): string | null {
  const typed = /^\/(\S*)$/.exec(draft)
  return typed ? typed[1]! : null
}

/** The commands whose name holds `typed`, those that start with it first, else as the agent listed them. */
export function matching(commands: AgentCommand[], typed: string): AgentCommand[] {
  const wanted = typed.toLowerCase()
  const holding = commands.filter((command) => command.name.toLowerCase().includes(wanted))
  const starts = (command: AgentCommand) => command.name.toLowerCase().startsWith(wanted)
  return [...holding.filter(starts), ...holding.filter((command) => !starts(command))]
}
