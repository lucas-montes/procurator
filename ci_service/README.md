# CI Service — Flake Checks

## What

Runs every check a repository's flake defines whenever someone pushes,
reusing the binary cache so already-built checks cost nothing. Captures each
check's log for users and sends events (failure, success, recovery) to
webhooks and the deploy target.

The target design is in [`context/specs/ci-service.md`](../context/specs/ci-service.md).

## Who builds

The CI host is not a build farm:

1. Users' machines build first: `pcr vcs repo push` builds the commit's
   checks and uploads them to the cache before pushing.
2. CI evaluates every check. One whose outputs are already in the cache (or
   on the host) is `cached` and nothing is built or downloaded.
3. A check missing from the cache goes to a remote builder for its system
   when the host's Nix config lists one (`nix.buildMachines`, filled by
   `services.procurator.ci.remoteBuilders`; builders are set up with
   `services.procurator.ciBuilder`, see `nix/modules/SERVICE_MODULES.md`),
   and is built on the CI host otherwise.
4. A check whose system nobody can build is `skipped`.

## Layout

Hexagonal, like `cache`:

- `src/domain/`: pure rules. `run.rs` covers run identity (`RepoName`,
  `CommitHash`, `GitRef`, `Pusher`), statuses and lifecycle; `check.rs`
  covers check statuses, "cached, else buildable, else skipped", and a run's
  outcome; `log.rs` tags each line with its origin (run, check, derivation);
  `repo_config.rs` holds the per-run settings and the service's caps;
  `event.rs` decides which events a finished run sends, and to whom;
  `deploy.rs` says which runs deploy and validates the deployment file;
  `access.rs` holds credentials and principals.
- `src/application/`: `Service` with every use case, `service/pipeline.rs`
  (the worker: claim, read config, evaluate, build and rerun, record),
  `service/logs.rs` (stored and followed logs), `service/live.rs` (the
  per-run broadcast), `service/outbox.rs` (the dispatcher),
  `service/retention.rs` (log expiry), and `ports.rs` (`RunStore`,
  `Builder`, `ConfigSource`, `Notifier`, `SecretStore`, `Access`, `Clock`).
- `src/adapters/`: `sqlite.rs` and `memory.rs` (run stores, both held to
  `store_contract.rs`), `nix/` (`NixBuilder`: `nix-eval-jobs`, one
  `nix build --keep-going`, internal-json log parsing, buildable systems from
  `nix config show`; `GitConfigSource`: `procurator.nix` by blob id),
  `webhook.rs` (`WebhookNotifier`, `HttpDeploy`), `notify.rs` (`Routed`:
  webhooks and deploys to their own adapters), `secrets.rs` (`FileSecretStore`),
  `access.rs` (`OpenAccess`), `system.rs` (clock), `http/` (axum router).
- `src/runtime.rs`: the production wiring (SQLite, Nix, git, webhooks,
  workers, dispatcher, log retention) from a `Config`, an `Access` adapter
  and a deploy adapter. `src/main.rs` (standalone) and the root `procurator` crate (the
  monolith: CI, repohub and cache in one process) both start it.

## HTTP API

| Method | Path | |
|---|---|---|
| POST | `/api/runs` | `{"repo":"owner/name","commit":"<sha>","ref":"refs/heads/main","pusher":"git"}` → 202 `{"id":1,"status":"queued"}` |
| GET | `/api/runs/{id}` | one run |
| GET | `/api/runs/{id}/checks` | checks with status, drv path and the machine that built them |
| GET | `/api/runs/{id}/checks/{attr}/log` | one check's log with the dependencies it needed built in the run, plain text, lines prefixed `<name>> ` |
| GET | `/api/runs/{id}/log` | the whole run, dependency builds included, lines prefixed `<name>> ` |
| GET | `/api/repos/{owner}/{name}/runs?before=<id>&limit=<n>` | newest first, default 20, max 100 |
| GET | `/health` | `OK` |

The post-receive hook lives in `nix/modules/repohub/post-receive`.

Add `?follow=true` to either log endpoint for Server-Sent Events: the
stored lines, then each line as it is built, then an `end` event when the
run ends. Each event's `data` is `{"seq","origin","text"}` (origin is the
derivation or check name, empty for the run) and its `id` is `seq`.

Reads take an optional `Authorization: Bearer <token>`; an invalid one is
401, a caller without access to the repo gets 403. The access check is in
the application layer (the `Access` port), so in-process callers get the
same answers. For now both binaries use `OpenAccess`: repohub has no
credentials or permissions yet, so everyone reads everything.

## Run locally

Needs `nix`, `nix-store` and `nix-eval-jobs` on `PATH`, and bare repos at
`<repos_base_path>/<owner>/<name>.git`.

```bash
cargo run -p ci_service -- --config ./ci_service/example.toml
```

## Cache

With `cache_base_url` set, what a run built (checks and their
dependencies, never downloaded paths) is pushed with `nix copy` using the
token in `netrc_file`. It goes to the owner's private cache
(`private-<owner>`) unless the default branch's `procurator.nix` sets
`ci.cache = "public"`. A failed push is a warning in the run log, not a
failed run. The token needs `write` on the public cache and on each owner's
private cache, and each private cache must exist on the cache service.

## Webhooks

The default branch's `procurator.nix` lists webhooks by secret name:

```nix
{ ci.webhooks = [ { urlSecret = "slack"; events = [ "run.failed" "run.recovered" ]; } ]; }
```

`secrets_file` maps names to URLs (`slack = "https://hooks…"`), so no URL
is ever committed. Events: `run.queued`, `run.started`, `check.passed`,
`check.failed`, `check.skipped`, `run.succeeded` (with output paths),
`run.failed`, `run.recovered`, `run.errored`. Each is a JSON POST
(`{"kind","run","repo","commit","ref","status","check"?,"outputs"?}`),
queued in the database with the run's result and retried with backoff
(30 s, 2 min, 10 min, 30 min) before giving up.

## Repo config: `procurator.nix`

Optional, at the repo root, read from the commit being checked. It is a
plain attribute set evaluated on its own (no imports, no fetches), and
unknown keys fail the run:

```nix
{
  ci = {
    checks = "all";            # or [ "checks.x86_64-linux.unit" "packages.x86_64-linux.default" ]
    reruns = 0;                # failed checks are built again this many times
    timeouts = { check = 3600; silent = 600; run = 7200; };   # seconds
  };
}
```

The service caps these with `max_reruns`, `max_check_timeout` and
`max_run_timeout`. A check's `tries` shows how many builds it took, so
`passed` with `tries > 1` is a flaky check. The full schema (webhooks,
retention, cache, deploy) is in the spec.

## Deploy

On the default branch:

```nix
{ deploy = { url = "http://cp.lan:8090/deploy"; branch = "main"; }; }
```

A run on `deploy.branch` then also builds the flake's
`procuratorDeployment` (it shows as a check of that name): a derivation
whose output is a JSON file listing every VM to run (id, worker, kernel,
initrd, cmdline, disk, CPUs, memory). `procurator.libs.<system>.deployment`
writes it from `diskVm` systems; see `nix/examples/python-workload`. When
the run passes, the file is validated and the run's `run.succeeded`, with
the deployment, is delivered until it arrives: the monolith with a
`control_plane` config publishes it as the control plane's next generation
(the same file again changes nothing); otherwise it is POSTed to
`deploy.url`. A broken deployment fails the run and deploys nothing.

## Log retention

A finished run's logs are kept `ci.logRetentionDays` days (read from the
default branch), else `default_log_retention_days` (30), never more than
`max_log_retention_days` (365). An hourly task deletes expired lines; runs
and checks stay, so history and flakiness survive.

## Status

Build steps 1–8 of the spec are done: queued runs are worked by
`max_concurrent_runs` workers, `procurator.nix` picks the checks, reruns
and timeouts, every check is evaluated, cached or built, each check's log
(with its dependencies) is stored and can be followed live, what was built
is pushed to the cache, events reach the repo's webhooks, reads go through
the `Access` port, old logs expire, and the root `procurator` binary runs
CI, repohub and the cache (and optionally the control plane) in one
process. Step 7 added the NixOS options for remote builders, step 8 the
deploy. Not yet: real access control (repohub has no credentials), the
hook token.
