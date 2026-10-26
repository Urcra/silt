import { createContext } from 'react'
import type { QuestionAsked } from './api'

/** The questions the thread's running agent waits on, answerable where the transcript shows them,
 * and the tool calls it waits on the person's permission for. */
export const LiveQuestions = createContext<{ thread: string; agent: string; questions: QuestionAsked[]; awaiting: string[] } | null>(null)
