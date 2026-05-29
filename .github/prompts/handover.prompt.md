---
description: Capture the current task for handoff via the sce-handover-writer skill.
agent: shared-context-code
---

Load and follow the [`sce-handover-writer`](../skills/sce-handover-writer/SKILL.md) skill.

Optional input:
${input:context:Optional plan/task identifiers or notes about current state}

## Behavior
- Keep this prompt as thin orchestration; handover structure, naming, and content decisions stay owned by `sce-handover-writer`.
- Run `sce-handover-writer` to gather current task state, decisions made and rationale, open questions or blockers, and the next recommended step.
- Let `sce-handover-writer` create the handover in `context/handovers/`, using task-aligned naming such as `context/handovers/{plan_name}-{task_id}-{timestamp}.md` when the inputs support it.
- If required details are missing, infer only from current repo state, label assumptions clearly, then stop after reporting the exact handover path.
