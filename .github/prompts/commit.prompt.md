---
description: Propose atomic commit message(s) from staged changes via the sce-atomic-commit skill.
agent: shared-context-code
---

Load and follow the [`sce-atomic-commit`](../skills/sce-atomic-commit/SKILL.md) skill.

Optional commit context:
${input:notes:Optional notes to refine wording (leave empty to infer from the staged diff)}

## Behavior
- If the notes input is empty, treat the input as unstated and infer commit intent from the staged changes only.
- If notes are provided, treat them as optional commit context to refine message proposals.
- Keep this prompt as thin orchestration; staged-diff analysis, atomic split decisions, and message wording stay owned by `sce-atomic-commit`.
- Before running `sce-atomic-commit`, explicitly stop and prompt the user:

  > Please run `git add <files>` for all changes you want included in this commit.
  > Atomic commits should only include intentionally staged changes.
  > Confirm once staging is complete.

- After confirmation:
  - Classify staged diff scope (`context/`-only vs mixed `context/` + non-`context/`) and apply the context-guidance gate from `sce-atomic-commit`.
  - Run `sce-atomic-commit` to produce commit-message proposals and any needed split guidance.
- Do not create commits automatically; stop after returning proposed commit message(s) and split guidance when needed.
