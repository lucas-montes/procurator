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
   when the host's Nix config lists one (`nix.buildMachines`), and is built
   on the CI host otherwise.
4. A check whose system nobody can build is `skipped`.

## Layout

Hexagonal, like `cache`:

- `src/domain/`: pure rules. `run.rs` covers run identity (`RepoName`,
  `CommitHash`, `GitRef`, `Pusher`), statuses and lifecycle; `check.rs`
  covers check statuses, "cached, else buildable, else skipped", and a run's
  outcome; `log.rs` tags each line with its origin (run, check, derivation);
  `repo_config.rs` holds the per-run settings and the service's caps.
- `src/application/`: `Service` with every use case, `service/pipeline.rs`
  (the worker: claim, read config, evaluate, build and rerun, record), and
  `ports.rs` (`RunStore`, `Builder`, `ConfigSource`, `Clock`).
- `src/adapters/`: `sqlite.rs` and `memory.rs` (run stores, both held to
  `store_contract.rs`), `nix/` (`NixBuilder`: `nix-eval-jobs`, one
  `nix build --keep-going`, internal-json log parsing, buildable systems from
  `nix config show`; `GitConfigSource`: `procurator.nix` by blob id),
  `system.rs` (clock), `http/` (axum router).
- `src/main.rs`: composition root for a standalone deployment. In the
  monolith, other crates hold `ci_service::application::Service` and call it
  directly.

## HTTP API

| Method | Path | |
|---|---|---|
| POST | `/api/runs` | `{"repo":"owner/name","commit":"<sha>","ref":"refs/heads/main","pusher":"git"}` → 202 `{"id":1,"status":"queued"}` |
| GET | `/api/runs/{id}` | one run |
| GET | `/api/runs/{id}/checks` | checks with status, drv path and the machine that built them |
| GET | `/api/runs/{id}/checks/{attr}/log` | one check's log, plain text |
| GET | `/api/runs/{id}/log` | the whole run, dependency builds included, lines prefixed `<name>> ` |
| GET | `/api/repos/{owner}/{name}/runs?before=<id>&limit=<n>` | newest first, default 20, max 100 |
| GET | `/health` | `OK` |

The post-receive hook lives in `nix/modules/repohub/post-receive`.

## Run locally

Needs `nix`, `nix-store` and `nix-eval-jobs` on `PATH`, and bare repos at
`<repos_base_path>/<owner>/<name>.git`.

```bash
cargo run -p ci_service -- --config ./ci_service/example.toml
```

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
retention, cache, deploy) is in the spec; those keys are validated now and
used by later steps.

## Status

Build steps 1–3 of the spec are done: queued runs are worked by
`max_concurrent_runs` workers, `procurator.nix` picks the checks, reruns
and timeouts, every check is evaluated, cached or built, and each check's
log is stored. Not yet: pushing built outputs to the cache (step 4),
notifications (step 5), live log streaming and access control (step 6).
