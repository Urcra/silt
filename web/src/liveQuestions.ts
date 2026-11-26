import { createContext } from 'react'
import type { QuestionAsked } from './api'

/** The questions the thread's running agent waits on, answerable where the transcript shows them,
 * the tool calls it waits on the person's permission for, and those Auto's check is judging (ADR 0132). */
export const LiveQuestions = createContext<{
  thread: string
  agent: string
  questions: QuestionAsked[]
  awaiting: string[]
  checking: string[]
} | null>(null)
