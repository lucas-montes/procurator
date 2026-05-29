---
name: sce-plan-review
description: |
  Reviews an existing SCE plan file (a Markdown checklist in `context/plans/`) to identify the next unchecked task, surface blockers or ambiguous acceptance criteria, and produce an explicit readiness verdict before implementation begins. Use when the user wants to continue a plan, resume work, pick the next step, or check what remains in an active plan — e.g. "continue the plan", "what's next?", "resume work on the plan", "review my plan and prepare the next task".
argument-hint: "[plan name] [task id]"
---

# sce-plan-review

## What I do
- Continue execution from an existing plan in `context/plans/`.
- Read the selected plan and identify the next task from the first unchecked checkbox.
- Ask focused questions for anything not clear enough to execute safely.

## How to run this
- Use this skill when the user asks to continue a plan or pick the next task.
- If `context/` is missing, ask once: "`context/` is missing. Bootstrap SCE baseline now?"
  - If yes, create baseline via [sce-bootstrap-context](../sce-bootstrap-context/SKILL.md) and continue.
  - If no, stop and explain SCE workflows require `context/`.
- Read `context/context-map.md`, `context/overview.md`, and `context/glossary.md` before broad exploration.
- Resolve plan target:
  - If a plan path argument exists, use it.
  - If multiple plans exist and no explicit path is provided, ask the user to choose.
- Collect:
  - completed tasks
  - next task
  - blockers, ambiguity, and missing acceptance criteria
- Prompt the user to resolve unclear points before implementation.
- Confirm scope explicitly for this session: one task by default unless the user requests multi-task execution.

## Plan file format
SCE plans are Markdown files stored in `context/plans/`. Tasks are tracked as checkboxes:

```markdown
# Plan: Add user authentication

## Tasks
- [x] Scaffold auth module
- [x] Add password hashing utility
- [ ] Implement login endpoint        <- next task (first unchecked)
- [ ] Write integration tests
- [ ] Update context/current-state.md
```

The first unchecked `- [ ]` item is the next task to review and prepare.

## Rules
- Do not auto-mark tasks complete during review.
- Keep continuation state in the plan markdown itself.
- Treat `context/plans/` as active execution artifacts; completed plans are disposable and not a durable context source.
- If durable history is needed, record it in current-state context files and/or `context/decisions/` instead of completed plan files.
- Keep implementation blocked until decision alignment on unclear points.
- If plan context is stale or partial, continue with code truth and flag context updates.

## Expected output

Produce a structured readiness summary after review:

```
## Plan Review — [plan filename]

**Completed tasks:** 2 of 5
**Next task:** Implement login endpoint

**Acceptance criteria:**
- POST /auth/login returns JWT on success
- Returns 401 on invalid credentials

**Issues found:**
- Blocker: JWT secret source not specified (env var? config file?)
- Ambiguity: Should failed attempts be rate-limited in this task or a later one?

**ready_for_implementation: no**

**Required decisions before proceeding:**
1. Confirm JWT secret source
2. Confirm rate-limiting scope
```

When all issues are resolved:

```
**ready_for_implementation: yes**
Proceeding with: Implement login endpoint
```

- Explicit readiness verdict: `ready_for_implementation: yes|no`.
- If not ready, explicit issue categories: blockers, ambiguity, missing acceptance criteria.
- Explicit user-aligned decisions needed to proceed to implementation.
- Explicit user confirmation request that the task is ready for implementation when unresolved issues remain.

## Related skills
- [sce-bootstrap-context](../sce-bootstrap-context/SKILL.md) — creates the `context/` baseline required by this skill.
- [sce-task-execution](../sce-task-execution/SKILL.md) — runs after the readiness verdict is `yes`.
