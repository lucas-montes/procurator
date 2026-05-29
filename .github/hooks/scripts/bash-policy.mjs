#!/usr/bin/env node
// VS Code PreToolUse hook: SCE bash-tool policy.
//
// Port of .opencode/plugins/sce-bash-policy.ts + bash-policy/runtime.ts.
//
// Wiring (https://code.visualstudio.com/docs/copilot/customization/hooks):
// - stdin: JSON object with at minimum { tool_name, tool_input, ... }.
// - stdout: optional JSON; to deny use
//     {"hookSpecificOutput":{"hookEventName":"PreToolUse",
//      "permissionDecision":"deny","permissionDecisionReason":"<reason>"}}
// - Exit code 2 also blocks the tool call and surfaces stderr to the model.
//
// Policy config is resolved (in order, later overrides earlier) from:
//   1. $XDG_STATE_HOME/sce/config.json (Linux) /
//      ~/Library/Application Support/sce/config.json (macOS) /
//      %APPDATA%/sce/config.json (Windows)
//   2. <workspace>/.sce/config.json
// The config shape mirrors the OpenCode plugin:
//   {"policies":{"bash":{"presets":["forbid-git-all", ...],
//                       "custom":[{"id":"...","message":"...",
//                                  "match":{"argv_prefix":["..."]}}]}}}
//
// Presets are loaded from ./bash-policy-presets.json (same dir as this file).

import { readFileSync, promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ---------- VS Code tool surface ----------

const TERMINAL_TOOLS = new Set([
  "runCommands",
  "run_in_terminal",
  "terminalLastCommand",
  "bash",
]);

function readStdinSync() {
  try {
    return JSON.parse(readFileSync(0, "utf8"));
  } catch {
    return null;
  }
}

function extractCommand(payload) {
  if (!payload || typeof payload !== "object") return null;
  if (!TERMINAL_TOOLS.has(payload.tool_name)) return null;

  // VS Code tools (camelCase) vs Claude-style (snake_case) — try both.
  const input = payload.tool_input ?? {};
  const command =
    input.command ??
    input.commandLine ??
    input.cmd ??
    input.script ??
    null;

  return typeof command === "string" && command.length > 0 ? command : null;
}

function emitDeny(reason) {
  process.stdout.write(
    JSON.stringify({
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason: reason,
      },
    }),
  );
  process.exit(0);
}

// ---------- Ported runtime ----------

const ENV_ASSIGNMENT_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*=.*/;
const WRAPPER_BINARIES = new Set([
  "env",
  "/usr/bin/env",
  "command",
  "nohup",
  "sudo",
]);
const SHELL_BINARIES = new Set(["sh", "bash"]);
const SHELL_OPERATORS = new Set(["|", "&&", "||", ";", "&"]);

function isPlainObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function tokenizeShellCommand(command) {
  const tokens = [];
  let current = "";
  let quote = null;
  let escaping = false;
  let i = 0;

  const pushCurrent = () => {
    if (current.length > 0) {
      tokens.push(current);
      current = "";
    }
  };

  while (i < command.length) {
    const ch = command[i] ?? "";

    if (escaping) {
      current += ch;
      escaping = false;
      i += 1;
      continue;
    }

    if (ch === "\\" && quote !== "'") {
      escaping = true;
      i += 1;
      continue;
    }

    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
      i += 1;
      continue;
    }

    if (ch === '"' || ch === "'") {
      quote = ch;
      i += 1;
      continue;
    }

    if (/\s/.test(ch)) {
      pushCurrent();
      i += 1;
      continue;
    }

    if (ch === "&" || ch === "|" || ch === ";") {
      pushCurrent();
      const next = command[i + 1] ?? "";
      if ((ch === "&" || ch === "|") && next === ch) {
        tokens.push(ch + next);
        i += 2;
        continue;
      }
      tokens.push(ch);
      i += 1;
      continue;
    }

    current += ch;
    i += 1;
  }

  if (escaping || quote) return null;
  pushCurrent();
  return tokens;
}

function parseCommandSegments(command) {
  const tokens = tokenizeShellCommand(command);
  if (!tokens || tokens.length === 0) return null;
  const segments = [];
  let current = [];
  for (const token of tokens) {
    if (SHELL_OPERATORS.has(token)) {
      if (current.length > 0) {
        segments.push(current);
        current = [];
      }
    } else {
      current.push(token);
    }
  }
  if (current.length > 0) segments.push(current);
  return segments;
}

function dropLeadingEnvAssignments(argv) {
  while (argv.length > 0 && ENV_ASSIGNMENT_PATTERN.test(argv[0] ?? "")) {
    argv.shift();
  }
}

function extractNixCommandArgv(segment) {
  for (let i = 1; i < segment.length; i += 1) {
    const t = segment[i];
    if (t !== "-c" && t !== "--command") continue;
    const nested = segment.slice(i + 1);
    return nested.length > 0 ? nested : null;
  }
  return null;
}

function extractShellCommandPayload(segment) {
  for (let i = 1; i < segment.length; i += 1) {
    const t = segment[i];
    if (!t || t === "--") continue;
    if (t === "-c") return segment[i + 1] ?? null;
    if (t.startsWith("-") && t.includes("c")) return segment[i + 1] ?? null;
  }
  return null;
}

function unwrapNestedCommandSegments(segment) {
  const exe = segment[0];
  if (!exe) return null;
  if (exe === "nix") {
    const nested = extractNixCommandArgv(segment);
    return nested ? [nested] : null;
  }
  if (SHELL_BINARIES.has(exe)) {
    const payload = extractShellCommandPayload(segment);
    if (!payload) return null;
    return parseCommandSegments(payload);
  }
  return null;
}

function normalizeSegment(segment) {
  if (segment.length === 0) return [];
  const normalized = [...segment];
  dropLeadingEnvAssignments(normalized);

  while (normalized.length > 0) {
    const exe = normalized[0];
    if (exe === undefined || !WRAPPER_BINARIES.has(exe)) break;
    normalized.shift();
    dropLeadingEnvAssignments(normalized);
  }

  if (normalized.length === 0) return [];

  normalized[0] = path.basename(normalized[0] ?? "");

  const nested = unwrapNestedCommandSegments(normalized);
  if (!nested) return [normalized];

  const nestedNormalized = [];
  for (const seg of nested) nestedNormalized.push(...normalizeSegment(seg));
  return nestedNormalized.length > 0 ? nestedNormalized : [normalized];
}

function argvStartsWith(argv, prefix) {
  if (prefix.length > argv.length) return false;
  return prefix.every((t, i) => argv[i] === t);
}

function comparePolicyPriority(left, right) {
  if (left.argvPrefix.length !== right.argvPrefix.length) {
    return right.argvPrefix.length - left.argvPrefix.length;
  }
  if (left.source !== right.source) {
    return left.source === "custom" ? -1 : 1;
  }
  return left.order - right.order;
}

function selectMatchingPolicy(activePolicies, normalizedArgv) {
  let best = null;
  for (const policy of activePolicies) {
    if (!argvStartsWith(normalizedArgv, policy.argvPrefix)) continue;
    if (!best || comparePolicyPriority(policy, best) < 0) best = policy;
  }
  return best;
}

// ---------- Config + preset loading ----------

function resolveGlobalConfigRoot() {
  const platform = process.platform;
  if (platform === "linux") {
    return (
      process.env.XDG_STATE_HOME ?? path.join(os.homedir(), ".local", "state")
    );
  }
  if (platform === "darwin") {
    return path.join(os.homedir(), "Library", "Application Support");
  }
  if (platform === "win32") return process.env.APPDATA ?? null;
  return (
    process.env.XDG_STATE_HOME ??
    process.env.XDG_DATA_HOME ??
    path.join(os.homedir(), ".local", "state")
  );
}

function getConfigSearchPaths(worktree) {
  const out = [];
  const root = resolveGlobalConfigRoot();
  if (root) out.push(path.join(root, "sce", "config.json"));
  out.push(path.join(worktree, ".sce", "config.json"));
  return out;
}

async function readJSON(file) {
  try {
    const raw = await fs.readFile(file, "utf8");
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function parseCustomPolicy(value) {
  if (!isPlainObject(value)) return null;
  if (!isPlainObject(value.match)) return null;
  const argvPrefix = value.match.argv_prefix;
  if (
    typeof value.id !== "string" ||
    value.id.length === 0 ||
    typeof value.message !== "string" ||
    value.message.length === 0 ||
    !Array.isArray(argvPrefix) ||
    argvPrefix.length === 0 ||
    argvPrefix.some((t) => typeof t !== "string" || t.length === 0)
  ) {
    return null;
  }
  return {
    id: value.id,
    message: value.message,
    argvPrefix,
    source: "custom",
    order: 0,
  };
}

function extractBashPolicyConfig(parsed) {
  if (!isPlainObject(parsed)) return null;
  const policies = parsed.policies;
  if (!isPlainObject(policies)) return null;
  const bash = policies.bash;
  if (!isPlainObject(bash)) return null;

  const presets = Array.isArray(bash.presets)
    ? bash.presets.filter((v) => typeof v === "string")
    : undefined;
  const custom = Array.isArray(bash.custom)
    ? bash.custom.map(parseCustomPolicy).filter((v) => v !== null)
    : undefined;

  return { presets, custom };
}

async function loadResolvedBashPolicyConfig(worktree) {
  let resolved = null;
  for (const cfgPath of getConfigSearchPaths(worktree)) {
    const parsed = await readJSON(cfgPath);
    if (!parsed) continue;
    const extracted = extractBashPolicyConfig(parsed);
    if (!extracted) continue;
    if (extracted.presets) {
      resolved = resolved ?? {};
      resolved.presets = extracted.presets;
    }
    if (extracted.custom) {
      resolved = resolved ?? {};
      resolved.custom = extracted.custom;
    }
  }
  if (!resolved) return null;
  const presets = resolved.presets ?? [];
  const custom = resolved.custom ?? [];
  if (presets.length === 0 && custom.length === 0) return null;
  return { presets, custom };
}

async function loadPresetCatalog() {
  const catalogPath = path.join(__dirname, "bash-policy-presets.json");
  const parsed = await readJSON(catalogPath);
  if (!parsed || !Array.isArray(parsed.presets)) return { presets: [] };
  return parsed;
}

function buildActivePolicies(policyConfig, presetCatalog) {
  const order = new Map();
  for (const [i, p] of presetCatalog.presets.entries()) order.set(p.id, i);

  const presetPolicies = [];
  for (const presetId of policyConfig.presets) {
    const idx = order.get(presetId);
    if (idx === undefined) continue;
    const preset = presetCatalog.presets[idx];
    for (const argvPrefix of preset.match.argv_prefixes) {
      presetPolicies.push({
        id: preset.id,
        message: preset.message,
        argvPrefix,
        source: "preset",
        order: idx,
      });
    }
  }

  const customPolicies = policyConfig.custom.map((p, i) => ({
    ...p,
    source: "custom",
    order: i,
  }));

  return [...presetPolicies, ...customPolicies];
}

// ---------- Main ----------

async function main() {
  const payload = readStdinSync();
  const command = extractCommand(payload);
  if (!command) process.exit(0);

  const worktree = payload?.cwd ?? process.cwd();

  const segments = parseCommandSegments(command);
  if (!segments || segments.length === 0) process.exit(0);

  const policyConfig = await loadResolvedBashPolicyConfig(worktree);
  if (!policyConfig) process.exit(0);

  const presetCatalog = await loadPresetCatalog();
  const active = buildActivePolicies(policyConfig, presetCatalog);
  if (active.length === 0) process.exit(0);

  for (const segment of segments) {
    for (const normalizedArgv of normalizeSegment(segment)) {
      if (normalizedArgv.length === 0) continue;
      const match = selectMatchingPolicy(active, normalizedArgv);
      if (match) {
        emitDeny(
          `Blocked by SCE bash-tool policy '${match.id}': ${match.message}`,
        );
      }
    }
  }

  process.exit(0);
}

main().catch((err) => {
  process.stderr.write(`bash-policy hook failed: ${err?.message ?? err}\n`);
  // Fail open: don't block tool execution on hook bugs.
  process.exit(0);
});
