import { createContext } from 'react'

/** Inside Settings, a page draws its title as a section heading under Settings' own header (ADR 0044). */
export const InSettings = createContext(false)

/** A Settings section's heading, the same in every section. */
export const SETTINGS_HEADING = 'text-heading font-medium'
