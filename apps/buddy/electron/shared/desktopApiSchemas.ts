import type { LexoraConfigPatch } from './desktopApi'
import { z } from 'zod'

export const lexoraConfigPatchSchema: z.ZodType<LexoraConfigPatch> = z.object({
  desktop: z.object({
    language: z.enum(['zh-CN', 'en-US']).optional(),
    launchAtLogin: z.boolean().optional(),
    theme: z.enum(['system', 'light', 'dark']).optional(),
  }).strict().optional(),
  agent: z.object({
    codex: z.object({
      defaultModel: z.string().optional(),
      reasoningEffort: z.string().trim().min(1).optional(),
    }).strict().optional(),
  }).strict().optional(),
}).strict()
