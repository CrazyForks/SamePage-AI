---
name: lexora-buddy-host
description: Use when the native Lexora Buddy desktop runtime needs host-owned desktop-pet actions without shell commands or IPC scripts.
---

# Lexora Buddy Host

This host-only skill is injected by the native Lexora Buddy desktop app. The Buddy host owns local desktop capabilities. Do not run commands, do not call Node scripts, and do not connect to sockets.

External clients use separate distributable skills. This built-in host skill must not include script, socket, launch, or installation instructions.

## Protocol

When the user asks for non-text Buddy desktop feedback, include exactly one hidden host action block in the assistant answer:

```txt
<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":2500}},"reason":"user_requested_pet_feedback"}</lexora_buddy_host_action>
```

Then answer the user naturally in text. The hidden block is consumed by Buddy and removed from the visible transcript.

## Actions

Use only `macroIntent`. Buddy plans choreographed pet behavior internally. Do not output direct `animation`, `move`, or `sequence` actions from this built-in host skill. Do not output raw `beatPlan`, `timelinePlan`, or `step` DSL; those are Buddy internal planner/runtime details.

```txt
<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":2500}},"reason":"user_requested_dance"}</lexora_buddy_host_action>
```

## Fields

- `version`: always `1`.
- `action`: required, exactly `macroIntent`.
- `intent`: required for macroIntent actions. Shape: `{"macroId":"dance","params":{"durationMs":2500}}`.
- `priority`: optional, exactly one of `background`, `normal`, `high`, `urgent`. Treat it as a constrained hint; Buddy derives action-log priority from runtime `triggerSource`. Only `urgent` is promoted to the `attentionSystem` trigger source, and model output must never request `criticalInteraction`.
- `reason`: optional 1..120 ASCII snake_case reason.

Do not add whitespace around string enum values. Buddy validates `action`, `priority`, and `reason` exactly and does not trim or normalize them.

## Macro Intents

Use one of these values:

- `celebrate` with `{}`
- `dance` with `{"durationMs":2500}`
- `lieDown` with `{}`
- `patrolAroundScreen` with `{"loops":2}`
- `reassure` with `{}`
- `sad` with `{}`
- `thinking` with `{}`
- `working` with `{}`
- `curious` with `{}`
- `awaitApproval` with `{}`
- `getUp` with `{"side":"left"}` or `{"side":"right"}`
- `peekFromEdge` with `{"edge":"left"}` using `left`, `right`, `top`, or `bottom`
- `peekBehindWindow` with `{"windowSelector":{"kind":"activeWindow"},"edge":"auto","reveal":"head","durationMs":1500}`. Prefer `auto` to let runtime choose the best active-window edge.
- `cast` with `{}`

## Parameter Limits

- dance `durationMs`: integer 1000..30000.
- patrolAroundScreen `loops`: integer 1..4.
- peekBehindWindow `durationMs`: integer 500..15000.
- peekBehindWindow `edge`: prefer `auto` to let runtime choose the best active-window edge. Use explicit `left`, `right`, `top`, or `bottom` only when the user names a side.
- peekFromEdge `edge`: one of `left`, `right`, `top`, `bottom`.

## Mapping

- dance as choreographed behavior: macroIntent `dance`
- celebrate, success, complete: macroIntent `celebrate`
- comfort, reassure: macroIntent `reassure`
- sad, upset, disappointed: macroIntent `sad`
- thinking, pondering, considering: macroIntent `thinking`
- working, focusing, busy: macroIntent `working`
- curious, interested, inspecting: macroIntent `curious`
- waiting for user approval or confirmation: macroIntent `awaitApproval`
- sleep, rest, quiet: macroIntent `lieDown`
- stand up, recover after a fall: macroIntent `getUp` with `side:"left"` or `side:"right"` when the recovery side is known
- patrol, run around the screen: macroIntent `patrolAroundScreen`
- peek from an edge: macroIntent `peekFromEdge`
- peek behind the active window: macroIntent `peekBehindWindow` with `edge:"auto"`
- cast, perform magic, dramatic effect: macroIntent `cast`

For behavior that is not represented by a macro intent, answer normally without a hidden host action block. Do not fall back to shell commands.
