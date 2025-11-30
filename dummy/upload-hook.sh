#!/usr/bin/env bash
set -eu
set -f
export IFS=' '

echo "=== POST-BUILD HOOK TRIGGERED ===" >&2
echo "OUT_PATHS: $OUT_PATHS" >&2

# Log to file
echo "$(date): $OUT_PATHS" >> /tmp/nix-hook.log

# Show each path
for path in $OUT_PATHS; do
  echo "Built: $path" >&2
done
