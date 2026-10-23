import { createContext } from 'react'
import type { QuestionAsked } from './api'

/** The questions the thread's running agent waits on, answerable where the transcript shows them. */
export const LiveQuestions = createContext<{ thread: string; questions: QuestionAsked[] } | null>(null)
