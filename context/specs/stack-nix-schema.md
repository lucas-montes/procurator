# Stack Nix Schema — Minimal

Minimal Nix schema for declaring services in a flake.

User-facing schema (minimal)
----------------------------

In `flake.nix` expose an `outputs.stack.services` attribute set:

```nix
{
  outputs = { self, pkgs, ... }: {
    stack = {
      services = {
        <name> = {
          cmd = "command";                    # Required: string (shell) or list (exec form)
          src = ./relative/path;              # Optional: working directory (defaults to repo root)
          ports = [ 8080 5432 ];              # Optional: list of host ports
          dependsOn = [ "service_name" ];     # Optional: list of service names to start first
          oneShot = false;                    # Optional: boolean, exit on completion (default: false)
          restart = "on-failure";             # Optional: "never" | "on-failure" | "always"
        };
      };
    };
  };
}
```

Field definitions
-----------------

- `cmd` (required): Command to execute. Can be:
  - String (shell): `"cargo run"` — executed via shell
  - Array (exec): `[ "cargo" "run" ]` — direct execution (preferred)

- `src` (optional): Working directory for the service. If omitted, repo root is used. Can be a path or flake input reference.

- `ports` (optional): List of TCP ports the service listens on (for validation and debugging). No automatic mapping.

- `dependsOn` (optional): List of service names that must start before this service starts. Creates a DAG; cycles are invalid.

- `oneShot` (optional, default: false): If true, the service is run synchronously and must complete. Failure aborts stack startup.

- `restart` (optional, default: "on-failure" for services, "never" for oneShot):
  - `"never"`: do not restart if it exits
  - `"on-failure"`: restart if exit code != 0
  - `"always"`: always restart

JSON representation (from `nix eval --json .#stack.services`)
-----------------------------------------------------------

The `nix eval` output is a flat JSON object mapping service names to service objects:

```json
{
  "db": {
    "cmd": ["postgres", "-D", "/var/lib/postgres"],
    "src": "/path/to/db",
    "ports": [5432],
    "dependsOn": [],
    "oneShot": false,
    "restart": "always"
  },
  "migrate": {
    "cmd": ["bash", "migrate.sh"],
    "src": "/path/to/migrate",
    "ports": [],
    "dependsOn": ["db"],
    "oneShot": true,
    "restart": "never"
  }
}
```

Validation rules
----------------

- `cmd` is required for all services.
- `ports` must be unique across services (no collisions).
- `dependsOn` must not have cycles.
- `restart` must be one of: `never`, `on-failure`, `always`.
- All service names in `dependsOn` must exist in the services map.

Example flake
-------------

Minimal example with two services (db + api):

```nix
{
  description = "example stack";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      stack = {
        services = {
          db = {
            cmd = [ "${pkgs.postgresql}/bin/postgres" "-D" "/var/lib/postgres" ];
            ports = [ 5432 ];
            restart = "always";
          };
          api = {
            cmd = "cargo run";
            src = ./api;
            ports = [ 8080 ];
            dependsOn = [ "db" ];
          };
        };
      };
    };
}
```

Runtime behavior
----------------

When `pcr stack up` is called:
1. `pcr` runs `nix eval --json .#stack.services` to fetch the services definition.
2. Validates the services (cycles, ports, missing required fields).
3. Computes a topological sort based on `dependsOn`.
4. For each service in order:
   - Changes to `src` directory if provided.
   - Spawns the process with the declared `cmd`.
   - Redirects stdout/stderr to a shared log aggregator.
   - All logs are prefixed with `[service_name]` and printed to the terminal.
5. Waits for Ctrl-C; on signal, stops all services in reverse order.

Implementation notes
--------------------

- No health checks in minimal v1; services start based on `dependsOn` ordering only.
- No restart loop logic in v1; services run once and can be manually restarted via `pcr stack restart <name>`.
- Logs are streamed only; not persisted to disk.
- Environment variables and secrets are inherited from the calling shell (no secret injection in v1).
