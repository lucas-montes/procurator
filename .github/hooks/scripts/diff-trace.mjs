#!/usr/bin/env node
// Stub: VS Code PostToolUse hook port of .opencode/plugins/sce-agent-trace.ts.
//
// Wiring contract (see https://code.visualstudio.com/docs/copilot/customization/hooks):
// - stdin: JSON with { tool_name, tool_input, tool_response, sessionId, ... }.
// - stdout: optional JSON; non-blocking by default.
// - exit code 0 (or any non-2 code) keeps the agent flowing.
//
// VS Code has no native `session.diff` event. The closest approximation is to
// react to edit-producing tools here, derive a diff (for example via `git diff
// --staged` or by recording the tool input/response), and forward to the
// existing `sce hooks diff-trace` CLI.

import { readFileSync } from "node:fs";
import { spawn } from "node:child_process";

let payload;
try {
  payload = JSON.parse(readFileSync(0, "utf8"));
} catch {
  process.exit(0);
}

const EDIT_TOOLS = new Set([
  "editFiles",
  "applyPatch",
  "create_file",
  "replace_string_in_file",
  "multi_replace_string_in_file",
]);

if (!EDIT_TOOLS.has(payload?.tool_name ?? "")) {
  process.exit(0);
}

const sessionId = payload?.sessionId ?? "unknown";
const diff = JSON.stringify(
  {
    tool_name: payload.tool_name,
    tool_input: payload.tool_input,
    tool_response: payload.tool_response,
  },
  null,
  2,
);

const child = spawn("sce", ["hooks", "diff-trace"], {
  stdio: ["pipe", "ignore", "inherit"],
});

child.on("error", () => process.exit(0)); // soft fail if `sce` is not on PATH
child.stdin.end(`${JSON.stringify({ sessionID: sessionId, diff, time: Date.now() })}\n`);
