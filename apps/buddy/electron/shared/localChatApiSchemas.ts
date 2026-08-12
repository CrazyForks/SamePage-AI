import { z } from 'zod'

const optionalLimitSchema = z.number().int().positive().optional()
const nullableStringSchema = z.string().nullable().optional()

const conversationSeedSchema = z.object({
  scope: z.enum(['global', 'project']),
  projectRoot: z.string().nullable(),
  title: z.string().nullable(),
  sourceConversationId: z.string().nullable(),
  forkedFromMessageId: z.string().nullable(),
  sourceRunId: z.string().nullable(),
}).strict()

const attachmentSchema = z.object({
  attachmentId: z.string().min(1),
}).strict()

const contextItemSchema = z.object({
  description: z.string().nullable(),
  kind: z.enum(['slashCommand', 'skill', 'plugin', 'file']),
  label: z.string().min(1),
  path: z.string().nullable(),
  value: z.string().min(1),
}).strict()

const codexInputSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('text'),
    text: z.string(),
    text_elements: z.array(z.object({
      byteRange: z.object({
        start: z.number().int().nonnegative(),
        end: z.number().int().nonnegative(),
      }).strict(),
      placeholder: z.string().nullable(),
    }).strict()),
  }).strict(),
  z.object({
    type: z.literal('image'),
    detail: z.enum(['auto', 'low', 'high']).optional(),
    url: z.string().min(1).max(6 * 1024 * 1024).regex(
      /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+={0,2}$/,
    ),
  }).strict(),
  z.object({
    type: z.literal('localImage'),
    detail: z.string().min(1).optional(),
    path: z.string().min(1),
  }).strict(),
  z.object({
    type: z.literal('skill'),
    name: z.string().min(1),
    path: z.string().min(1),
  }).strict(),
  z.object({
    type: z.literal('mention'),
    name: z.string().min(1),
    path: z.string().min(1),
  }).strict(),
])

const runtimeStateSchema = z.object({
  status: z.enum(['stopped', 'starting', 'ready', 'restarting', 'offline', 'stopping']),
  pid: z.number().int().positive().nullable(),
  restartAttempt: z.number().int().nonnegative(),
  lastError: z.string().nullable(),
}).strict()

const codexStatusSchema = z.object({
  cliAvailable: z.boolean(),
  version: z.string().nullable(),
  loginStatus: z.enum(['logged_in', 'logged_out', 'unknown', 'unavailable']),
  appServerAvailable: z.boolean(),
  execJsonAvailable: z.boolean(),
  preferredProtocol: z.literal('codex_app_server'),
  activeProtocol: z.enum(['codex_app_server', 'codex_exec_json_fallback', 'unavailable']),
}).strict()

const claudeStatusSchema = z.object({
  cliAvailable: z.boolean(),
  version: z.string().nullable(),
  loginStatus: z.enum(['logged_in', 'logged_out', 'unknown', 'unavailable']),
  authMethod: z.string().nullable(),
  apiProvider: z.string().nullable(),
  printModeAvailable: z.boolean(),
  streamJsonAvailable: z.boolean(),
  memoryIsolationAvailable: z.boolean(),
  preferredProtocol: z.literal('claude_print_stream_json'),
  activeProtocol: z.enum(['status_only', 'unavailable']),
  executionEnabled: z.boolean(),
}).strict()

const localStateSchema = z.object({
  paths: z.object({
    dataDir: z.string().min(1),
    attachmentsDir: z.string().min(1),
    artifactsDir: z.string().min(1),
    cacheDir: z.string().min(1),
    conversationsDir: z.string().min(1),
    logDir: z.string().min(1),
    memoriesDir: z.string().min(1),
    runsDir: z.string().min(1),
    sqliteDir: z.string().min(1),
    databasePath: z.string().min(1),
  }).strict(),
  storage: z.object({
    databasePath: z.string().min(1),
    schemaVersion: z.number().int().nonnegative(),
  }).strict(),
}).strict()

const usageRuntimeSchema = z.enum(['codex', 'claude'])
const usageStatusSchema = z.enum(['available', 'empty', 'unavailable'])
const usageTotalsSchema = z.object({
  inputTokens: z.number().int().nonnegative(),
  outputTokens: z.number().int().nonnegative(),
  cacheCreationTokens: z.number().int().nonnegative(),
  cacheReadTokens: z.number().int().nonnegative(),
  totalTokens: z.number().int().nonnegative(),
  recordCount: z.number().int().nonnegative(),
}).strict()
const usageRecordSchema = usageTotalsSchema.omit({ recordCount: true }).extend({
  runtime: usageRuntimeSchema,
  date: z.string().nullable(),
  sessionId: z.string().nullable(),
  projectPath: z.string().nullable(),
  model: z.string().nullable(),
}).strict()
const usageSnapshotSchema = z.object({
  sources: z.array(z.object({
    runtime: usageRuntimeSchema,
    status: usageStatusSchema,
    source: z.string(),
    updatedAt: z.string().nullable(),
    message: z.string().nullable(),
  }).strict()),
  totals: usageTotalsSchema,
  records: z.array(usageRecordSchema),
  windows: z.array(z.object({
    runtime: usageRuntimeSchema,
    key: z.enum(['codex_5h_limit', 'codex_weekly_limit', 'claude_5h_limit', 'claude_weekly_limit']),
    status: usageStatusSchema,
    usedTokens: z.number().int().nonnegative().nullable(),
    percentage: z.number().int().min(0).max(100).nullable(),
    resetsAt: z.string().nullable(),
  }).strict()),
}).strict()

const projectSchema = z.object({
  root: z.string().min(1),
  name: z.string().min(1),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1),
}).strict()

const conversationSchema = z.object({
  id: z.string().min(1),
  activeBranchId: z.string().min(1),
  scope: z.enum(['global', 'project']),
  projectRoot: z.string().nullable(),
  title: z.string().nullable(),
  logPath: z.string().min(1),
  sourceConversationId: z.string().nullable(),
  forkedFromMessageId: z.string().nullable(),
  sourceRunId: z.string().nullable(),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1),
}).strict()

const messageAttachmentSchema = z.object({
  attachmentId: z.string().nullable(),
  dataUrl: z.string().nullable(),
  kind: z.enum(['image', 'text', 'binary']),
  mimeType: z.string().min(1),
  name: z.string().min(1),
  previewPath: z.string().nullable(),
  sizeBytes: z.number().int().nonnegative(),
}).strict()

const messageSchema = z.object({
  id: z.string().min(1),
  sessionId: z.string().nullable(),
  conversationId: z.string().nullable(),
  branchId: z.string().nullable(),
  runId: z.string().nullable(),
  parentMessageId: z.string().nullable(),
  versionGroupId: z.string().nullable(),
  versionIndex: z.number().int().nonnegative().nullable(),
  versionStatus: z.enum(['active', 'superseded']).nullable(),
  role: z.enum(['system', 'user', 'assistant', 'tool']),
  content: z.string(),
  attachments: z.array(messageAttachmentSchema),
  createdAt: z.string().min(1),
}).strict()

const runSchema = z.object({
  id: z.string().min(1),
  sessionId: z.string().nullable(),
  conversationId: z.string().nullable(),
  branchId: z.string().nullable(),
  triggeringMessageId: z.string().nullable(),
  intent: z.string().nullable(),
  logPath: z.string().nullable(),
  runtime: z.literal('codex'),
  cwd: z.string().nullable(),
  status: z.enum(['queued', 'running', 'completed', 'failed', 'cancelled']),
  externalThreadId: z.string().nullable(),
  externalRunId: z.string().nullable(),
  startedAt: z.string().min(1),
  completedAt: z.string().nullable(),
}).strict()

const runEventSchema = z.object({
  id: z.number().int().nonnegative(),
  runId: z.string().min(1),
  eventType: z.string().min(1),
  payload: z.json(),
  createdAt: z.string().min(1),
}).strict()

const approvalSchema = z.object({
  id: z.string().min(1),
  runId: z.string().nullable(),
  kind: z.string().min(1),
  status: z.enum(['pending', 'approved', 'denied', 'cancelled']),
  payload: z.json(),
  createdAt: z.string().min(1),
  resolvedAt: z.string().nullable(),
}).strict()

const contextOptionsSchema = z.object({
  files: z.array(contextItemSchema),
  plugins: z.array(contextItemSchema),
  skills: z.array(contextItemSchema),
}).strict()

const modelOptionSchema = z.object({
  runtime: z.literal('codex'),
  id: z.string().min(1),
  model: z.string().min(1),
  displayName: z.string().min(1),
  description: z.string().nullable(),
  isDefault: z.boolean(),
  defaultReasoningEffort: z.string().nullable(),
  supportedReasoningEfforts: z.array(z.object({
    reasoningEffort: z.string().min(1),
    description: z.string().nullable(),
  }).strict()),
  serviceTiers: z.array(z.object({
    id: z.string().min(1),
    name: z.string().min(1),
    description: z.string().nullable(),
  }).strict()),
  defaultServiceTier: z.string().nullable(),
}).strict()

const attachmentResponseSchema = messageAttachmentSchema.extend({
  attachmentId: z.string().min(1),
  text: z.string().nullable(),
}).strict()

const workspaceDraftAttachmentSchema = z.object({
  attachmentId: z.string().min(1),
  kind: z.enum(['image', 'text', 'binary']),
  mimeType: z.string().min(1),
  name: z.string().min(1),
  sizeBytes: z.number().int().nonnegative(),
}).strict()

const workspaceDraftSchema = z.object({
  attachments: z.array(workspaceDraftAttachmentSchema).max(16),
  composerContent: z.json().nullable(),
  content: z.string(),
  requestFingerprint: z.string().min(1).max(1_024).nullable(),
  requestId: z.string().min(1).max(128).nullable(),
  targetKey: z.string().min(1),
}).strict()

export const LOCAL_WORKSPACE_STATE_KEY = 'buddy.chat.workspace.v2' as const

export const localWorkspaceStateValueSchema = z.object({
  activeConversationId: z.string().nullable(),
  drafts: z.array(workspaceDraftSchema),
  projectRoot: z.string().nullable(),
  sidebarCollapsed: z.boolean(),
}).strict()

const workspaceSettingSchema = z.object({
  key: z.literal(LOCAL_WORKSPACE_STATE_KEY),
  value: localWorkspaceStateValueSchema,
  updatedAt: z.string().min(1),
}).strict()

const turnStartSchema = z.object({
  assistantMessage: messageSchema.nullable(),
  conversation: conversationSchema,
  intent: z.string().min(1),
  userMessage: messageSchema,
  run: runSchema.nullable(),
}).strict()

export const localChatResponseSchemas = {
  runtimeState: runtimeStateSchema,
  localState: localStateSchema,
  codexStatus: codexStatusSchema,
  claudeStatus: claudeStatusSchema,
  usageSnapshot: usageSnapshotSchema,
  modelOptions: z.array(modelOptionSchema),
  contextOptions: contextOptionsSchema,
  project: projectSchema,
  optionalProject: projectSchema.nullable(),
  projects: z.array(projectSchema),
  optionalWorkspaceSetting: workspaceSettingSchema.nullable(),
  workspaceSetting: workspaceSettingSchema,
  conversations: z.array(conversationSchema),
  deleted: z.boolean(),
  messages: z.array(messageSchema),
  runs: z.array(runSchema),
  run: runSchema,
  runEvents: z.array(runEventSchema),
  approvals: z.array(approvalSchema),
  approvalResolution: z.json(),
  attachments: z.array(attachmentResponseSchema),
  releasedAttachments: z.object({
    releasedAttachmentIds: z.array(z.string().min(1)),
  }).strict(),
  turnStart: turnStartSchema,
} as const

export const localChatSchemas = {
  attachmentPreview: z.object({
    path: z.string().min(1),
    mimeType: z.string().regex(/^image\//),
  }).strict(),
  limit: z.object({ limit: optionalLimitSchema }).strict(),
  codexContext: z.object({
    cwd: z.string().nullable(),
    fileQuery: nullableStringSchema,
  }).strict(),
  conversationId: z.object({ conversationId: z.string().min(1) }).strict(),
  conversationMessages: z.object({
    conversationId: z.string().min(1),
    limit: optionalLimitSchema,
  }).strict(),
  listRuns: z.object({
    sessionId: nullableStringSchema,
    conversationId: nullableStringSchema,
    limit: optionalLimitSchema,
  }).strict().refine(
    input => !(input.sessionId && input.conversationId),
    { message: 'sessionId and conversationId are mutually exclusive' },
  ),
  runId: z.object({ runId: z.string().min(1) }).strict(),
  runEvents: z.object({
    runId: z.string().min(1),
    afterId: z.number().int().nonnegative().nullable().optional(),
    limit: optionalLimitSchema,
  }).strict(),
  conversationEvents: z.object({
    conversationId: z.string().min(1),
    afterId: z.number().int().nonnegative().nullable().optional(),
    runLimit: optionalLimitSchema,
    eventLimit: optionalLimitSchema,
  }).strict(),
  listApprovals: z.object({
    status: nullableStringSchema,
    limit: optionalLimitSchema,
  }).strict(),
  approvalId: z.object({ approvalId: z.string().min(1) }).strict(),
  attachmentSelection: z.object({
    remainingCount: z.number().int().min(1).max(16),
  }).strict(),
  attachmentRelease: z.object({
    attachmentIds: z.array(z.string().min(1)).max(16),
  }).strict(),
  retainedAttachments: z.object({
    retainedAttachmentIds: z.array(z.string().min(1)),
  }).strict(),
  workspaceValue: z.object({ value: localWorkspaceStateValueSchema }).strict(),
  startTurn: z.object({
    requestId: z.string().min(1).max(128),
    conversationId: nullableStringSchema,
    conversationSeed: conversationSeedSchema.nullable().optional(),
    content: z.string(),
    cwd: z.string().nullable(),
    attachments: z.array(attachmentSchema).optional(),
    contextItems: z.array(contextItemSchema).optional(),
    inputs: z.array(codexInputSchema).optional(),
    modelSelection: z.object({
      runtime: z.literal('codex'),
      model: z.string().nullable(),
      serviceTier: z.string().nullable(),
      effort: z.string().nullable(),
    }).strict().nullable().optional(),
  }).strict(),
  runStateEvent: z.object({
    runId: z.string().min(1),
    sessionId: z.string().nullable(),
    eventId: z.number().int().nonnegative().nullable(),
    eventType: z.string().nullable(),
    status: z.enum(['queued', 'running', 'completed', 'failed', 'cancelled']).nullable(),
  }).strict(),
} as const

type DeepReadonly<T> = T extends ReadonlyArray<infer Item>
  ? ReadonlyArray<DeepReadonly<Item>>
  : T extends object
    ? { readonly [Key in keyof T]: DeepReadonly<T[Key]> }
    : T

export type LocalRuntimeSupervisorState = DeepReadonly<z.infer<typeof runtimeStateSchema>>
export type LocalStateStatus = DeepReadonly<z.infer<typeof localStateSchema>>
export type LocalCodexRuntimeStatus = DeepReadonly<z.infer<typeof codexStatusSchema>>
export type LocalClaudeRuntimeStatus = DeepReadonly<z.infer<typeof claudeStatusSchema>>
export type LocalUsageSnapshot = DeepReadonly<z.infer<typeof usageSnapshotSchema>>
export type LocalProject = DeepReadonly<z.infer<typeof projectSchema>>
export type LocalConversation = DeepReadonly<z.infer<typeof conversationSchema>>
export type LocalMessageAttachment = DeepReadonly<z.infer<typeof messageAttachmentSchema>>
export type LocalMessage = DeepReadonly<z.infer<typeof messageSchema>>
export type LocalRun = DeepReadonly<z.infer<typeof runSchema>>
export type LocalRunEvent = DeepReadonly<z.infer<typeof runEventSchema>>
export type LocalRunStateEvent = DeepReadonly<z.infer<typeof localChatSchemas.runStateEvent>>
export type LocalApproval = DeepReadonly<z.infer<typeof approvalSchema>>
export type LocalPromptContextOption = DeepReadonly<z.infer<typeof contextItemSchema>>
export type LocalCodexContextOptions = DeepReadonly<z.infer<typeof contextOptionsSchema>>
export type LocalRuntimeModelOption = DeepReadonly<z.infer<typeof modelOptionSchema>>
export type LocalAttachment = DeepReadonly<z.infer<typeof attachmentResponseSchema>>
export type LocalWorkspaceDraft = DeepReadonly<z.infer<typeof workspaceDraftSchema>>
export type LocalWorkspaceStateValue = DeepReadonly<z.infer<typeof localWorkspaceStateValueSchema>>
export type LocalCodexInput = z.infer<typeof codexInputSchema>
export type LocalStartTurnRequest = z.infer<typeof localChatSchemas.startTurn>
export type LocalTurnStart = DeepReadonly<z.infer<typeof turnStartSchema>>
export type LocalWorkspaceSetting = DeepReadonly<z.infer<typeof workspaceSettingSchema>>
