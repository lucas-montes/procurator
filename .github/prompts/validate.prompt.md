---
description: Finish an SCE plan with validation and cleanup via the sce-validation skill.
agent: shared-context-code
---

Load and follow the [`sce-validation`](../skills/sce-validation/SKILL.md) skill.

Input:
${input:plan:Plan name or path to validate}

## Behavior
- Keep this prompt as thin orchestration; validation scope, command selection, cleanup, and evidence formatting stay owned by `sce-validation`.
- Run `sce-validation` to execute the full validation phase for the targeted plan or change, including required checks, evidence capture, and cleanup expected by the skill.
- Let `sce-validation` decide pass/fail status and record any residual risks or unmet criteria.
- Stop after reporting the validation outcome and the location of any written validation evidence.
