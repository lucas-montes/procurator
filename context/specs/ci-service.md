# CI Service — Spec

Status: agreed on 2026-11-14. Build steps 1–8 are done (see "Build
order"); "Current state" at the end describes the scaffold they replaced.
Progress, acceptance-criteria status and the remaining work are tracked in
[`../plans/ci-service.md`](../plans/ci-service.md).

## Purpose

Run every check a repository's flake defines whenever someone pushes, as fast as
possible, by reusing the binary cache. Capture each check's log so users can
watch it live and read it later, and send events (failure, success, recovery)
to whoever cares through webhooks and a deploy URL. Any UI, repohub included,
reads results from the CI API.

Non-goals for now: editing the config from a UI (that means committing to the
repo), and the `deploy` section's schema beyond `url` and `branch` (what is
deployed comes from the flake, see "Deploy").

## Decisions

| Topic | Decision |
|---|---|
| What runs | Every check in `checks.<system>.*`, or the attribute list in the repo config. |
| Who builds | Users' machines are the primary builders: `pcr vcs repo push` builds the commit's checks and uploads them to the cache before pushing, so CI usually only evaluates and finds every output cached. A check missing from the cache goes to a remote builder for its system when one is configured and reachable (Nix `builders`, set in the host's Nix config). Otherwise the CI host builds it. A check whose system neither a remote builder nor the CI host can build is `skipped`. (Decided 2026-11-15.) |
| Concurrent pushes | Every push gets its own run. Runs never cancel each other. |
| Reuse | No "skip this commit" shortcut. Every run evaluates, and anything already in the cache counts as passed without building. Results users pushed from their machines are trusted. |
| Reruns, timeouts | Set in the repo config, capped by service-side limits. |
| Logs | Per check, streamed live and stored. Retention set per repo. Only users related to the repo can read them. |
| Notifications | One `Notifier` port. Adapters: webhook (per repo, selectable events) and deploy (`run.succeeded` with the run's deployment: in the monolith, published to the embedded control plane; otherwise POSTed to the repo's `deploy.url`). Repohub is not notified; it calls the CI API. |
| Deployment | Built and shipped as a monolith first: the root `procurator` binary composes `ci_service`, `repohub` and `cache` in one process, and ports are satisfied by direct function calls. The same ports have HTTP adapters, so the crates can be split into separate services later without touching domain or application code. |
| Auth | Lives entirely behind the `Access` port. In the monolith it calls repohub's application service directly. When split, it calls repohub's API or verifies a JWT. `ci_service` itself stores no users. |
| Dependency logs | Shown live at run level while building, then attached to every check that depends on them. |
| Delivery | Events go to an outbox table first. Each adapter delivers and retries on its own terms. |
| Cache | The CI service downloads and uploads using its own service token. Public repo → public cache. Private repo → that owner's private cache. |
| Config location | A self-contained `procurator.nix` at the repo root, in a procurator-specific format, shared later with deployment settings. |
| Config caching | Parsed config kept in memory, keyed by the file's Git blob id. |
| Architecture | Hexagonal (`domain` / `application` / `adapters`), like `cache` and `repohub`, so stores and integrations can be swapped. |
| Security | Locked-down evaluation now (see Security). One ephemeral VM per run later. |

## Repo config: `procurator.nix`

A plain attribute set at the repo root. It must not import anything (no
nixpkgs, no other files), so its Git blob id identifies it exactly and it stays
simple for a UI to edit. It is evaluated with `restrict-eval` and nothing
allowed, which enforces this. A missing file means all defaults. An invalid file
fails the run with a clear error and does not fall back to defaults.

```nix
{
  ci = {
    # "all" = every checks.<system>.*; or explicit attribute paths.
    checks = "all";
    # checks = [ "checks.x86_64-linux.unit" "packages.x86_64-linux.default" ];

    reruns = 0;                 # times a failed check is built again
    timeouts = {
      check = 3600;             # seconds without finishing, per derivation
      silent = 600;             # seconds without output, per derivation
      run = 7200;               # whole run
    };

    logRetentionDays = 30;
    cache = null;               # null = by repo visibility; or a cache name

    webhooks = [
      {
        urlSecret = "slack-alerts";   # name in the service's secret store, never the URL
        events = [ "run.failed" "check.failed" "run.recovered" ];
      }
    ];
  };

  deploy = {
    url = null;                 # null = no deploy; otherwise run.succeeded is POSTed here
                                # (the monolith with a control plane deploys in process
                                # and only reads it as "on")
    branch = "main";            # only runs on this branch trigger a deploy
  };
}
```

### Deploy

What a run deploys is a flake output, not config: with `deploy.url` set
(default branch) and the run on `deploy.branch`, the run also evaluates and
builds `procuratorDeployment`, shown as a check of that name. It must be a
derivation whose output is a JSON file:

```json
{"version": 1, "vms": [{"id": "web", "worker": "worker-1",
  "kernel": "/nix/store/…/bzImage", "initramfs": "/nix/store/…/initrd",
  "cmdline": "console=ttyS0 … init=/nix/store/…/init",
  "disk": "/nix/store/…/nixos.img", "cpus": 2, "memoryMiB": 1024}]}
```

`procurator.libs.<system>.deployment { vms.<id> = { worker; nixos; cpus;
memoryMiB; }; }` writes it from `diskVm` systems. The file references the
boot files, so building it builds them and the cache push carries them.
Every VM is checked (version 1; 1–256 VMs; unique ids of `a-z 0-9 -`; boot
files are store paths; 1–256 CPUs; 64 MiB–1 TiB): an invalid file, or a
deployment that was skipped or did not build, fails the run and nothing is
deployed. A missing `procuratorDeployment` fails its check. The deployment
is the whole desired state: the control plane replaces its generation with
it, and the file's store path is the intent, so deploying the same file
again changes nothing.

### Which commit's config applies

- **Per-run settings** (`ci.checks`, `ci.reruns`, `ci.timeouts`) come from the
  commit being checked, capped by the service limits.
- **Repo-level settings** (`ci.webhooks`, `ci.logRetentionDays`, `ci.cache`,
  `deploy`) come from the **default branch head** only, so a feature branch
  cannot reroute notifications, extend retention or trigger deploys.

### Resolution

1. `git rev-parse <commit>:procurator.nix` gives the blob id (or "missing").
2. If the in-memory map has that blob id, use the parsed struct. No Nix runs.
3. Otherwise read the blob, evaluate it to JSON in isolation, parse and
   validate it into `RepoConfig`, and store it in the map (bounded LRU).
4. Repo-level settings: one `RepoConfig` per repo for the default branch head,
   replaced when a push moves the default branch.

## Pipeline

```
post-receive ──► POST /api/runs ──► RunStore (queued)
                                        │
                            worker claims (UPDATE … RETURNING)
                                        │
            ConfigSource ──► resolve RepoConfig (in-memory by blob id)
                                        │
            Builder.evaluate ──► one Check per attribute
                                 (attr, system, drv, cache status)
                                        │
                ├─ cached ──────────────► cached (counts as passed), nothing downloaded
                ├─ no builder for system ► skipped
                └─ rest ──► Builder.build (one nix build --keep-going)
                                 │  remote builder for the system if configured,
                                 │  otherwise the CI host
                                 │  per-check log lines ──► LogSink (store + live)
                                 ├─ failed ──► rebuild up to `reruns`
                                 └─ passed ──► push outputs to the cache
                                        │
                              Events ──► outbox ──► Notifier adapters
```

### Steps

1. **Enqueue.** The hook sends `repo` (a name such as `owner/name`), `commit`,
   `ref` and pusher info. The service resolves the bare-repo path itself under
   `repos_base_path` and never accepts a path from the request.
2. **Claim.** `UPDATE runs SET status='running' … RETURNING *` in one
   statement. On startup, runs left `running` by a crash go back to `queued`.
   Up to `max_concurrent_runs` runs at once.
3. **Config.** As above.
4. **Evaluate.**
   `nix-eval-jobs --flake 'git+file://<repo>?rev=<sha>#checks' --check-cache-status --workers <n>`
   with the security options. For an explicit list, evaluate each attribute the
   same way. Output is one JSON line per attribute: `attr`, `system`,
   `drvPath`, `outputs`, `cacheStatus`, or an evaluation error, which becomes a
   failed check with the error as its log.
5. **Systems.** Compare each check's system with the systems available: the
   CI host's `system` and `extra-platforms` plus the systems of every machine
   in `builders` (inline or `@file`), all read from `nix config show --json`.
   Unmatched checks become `skipped: no builder for <system>`.
6. **Build.**
   `nix build --keep-going --no-link --print-out-paths --log-format internal-json --timeout <check> --max-silent-time <silent> '<drv>^*' …`
   for every check not cached and not skipped, in one invocation. Nix then
   builds in parallel, downloads what is cached, builds shared dependencies
   once, and offers each derivation to the remote builders of its system
   first, building on the CI host when none accepts and the host can build
   that system. The build-start event names the machine (`""` = CI host),
   which is stored on the check. Remote builders get
   `builders-use-substitutes = true` so they download dependencies from the
   cache instead of through the CI host. `internal-json` events
   carry the derivation being built. Map each log line to its check through the
   `drvPath` from step 4. A dependency's lines are streamed live at run level
   as "dependency `<name>`". When the run finishes they are attached to every
   check whose closure contains that dependency, so each check's stored log is
   complete on its own.
7. **Reruns.** Failed checks are built again, up to `reruns` times. Each try is
   recorded, so flaky checks show up as "failed then passed".
8. **Push.** `nix copy --to <cache-url>` the outputs that were built (not the
   ones downloaded), using the service token.
9. **Finish.** Write the run result and the events in one transaction (see
   Events).

The run timeout kills the `nix` processes and marks the remaining checks
`timed_out`.

## Domain

All in `ci_service/src/domain/`, no `tokio`, `axum` or `sqlx`.

- `Run`: id, repo, commit, ref, pusher, status, timestamps.
- `RunStatus`: `queued`, `running`, `passed`, `failed`, `timed_out`, `errored`
  (infrastructure fault, such as Nix missing or the database down).
- `Check`: run id, attribute, system, drv path, status, tries.
- `CheckStatus`: `queued`, `building`, `passed`, `cached`, `failed`, `skipped`,
  `timed_out`.
- `RepoConfig`, `RunSettings` (per-run part, already capped), `RepoSettings`
  (default-branch part).
- `Event`: kind plus a payload with everything a consumer needs, so adapters
  never call back into the service.

### Events

| Kind | When |
|---|---|
| `run.queued`, `run.started` | lifecycle |
| `check.failed`, `check.passed`, `check.skipped` | per check, after its last try |
| `run.succeeded` | all checks passed, cached or skipped. Payload includes the output paths. |
| `run.failed` | at least one check failed or timed out |
| `run.recovered` | default branch: previous run failed, this one succeeded |
| `run.errored` | infrastructure fault, not the repo's fault |

## Application ports

In `ci_service/src/application/ports.rs`. Async methods return
`impl Future<Output = …>`.

| Port | Responsibility | Adapters |
|---|---|---|
| `RunStore` | runs, checks, tries, log chunks, outbox, retention deletes | SQLite now. In-memory for tests. |
| `Access` | authenticate the credential an HTTP request carries into a principal. Can principal X read repo Y? What are the repo's visibility, owner and default branch? | monolith: wraps repohub's application service. Split: repohub's HTTP API, or a JWT verifier. |
| `ConfigSource` | `RepoConfig` for a commit (blob-id cache inside) | Git + `nix eval` |
| `SecretStore` | resolves secret names (webhook URLs) | TOML/file now |
| `Builder` | buildable systems, evaluate attributes, build drvs with streamed per-drv logs, push outputs | nix daemon (dispatching to remote builders from the host's Nix config) now, ephemeral VM later |
| `Notifier` | deliver one event | webhook, deploy (monolith: calls the control plane's application service; split: POST to `deploy.url`) |
| `Clock` | time, for timeouts and retention | system |

### Inbound side

The application exposes one service struct (`ci_service::Service`, like
`cache`'s) with every use case: enqueue a run, list runs, get a run, read or
follow a log. Callers reach it in one of two ways:

- **Monolith:** repohub, the CLI or any other crate in the process holds the
  `Service` and calls its methods directly.
- **Split:** the axum router in `adapters/http/` is a thin translation of HTTP
  requests to the same methods.

Both paths enforce access the same way, because the check is in the
application layer, not in the HTTP handlers.

### Monolith composition

`ci_service` never depends on `repohub`, `cache` or `control_plane`. The
in-process adapters live in the composition root (the root `procurator`
crate): small wrapper structs there implement `ci_service`'s port traits by
calling the other crates' services, e.g. `RepohubAccess` wraps
`repohub::RepositoryService` and implements `Access`. This keeps every crate
buildable and testable on its own, with no dependency cycles.

Three things stay out of process even in the monolith, because external
programs are involved:

- **The post-receive hook** is spawned by git as its own process. It reaches
  the monolith through `POST /api/runs` on localhost (or a Unix socket).
- **Nix talks to the cache over HTTP**, for both substituting and `nix copy`,
  so the cache's HTTP router is always mounted, even in the monolith.
- **The nix daemon** builds everything.

### Live logs

Live log streaming is **not** a `Notifier` concern. The application keeps one
in-process broadcast channel per running run. The HTTP adapter subscribes
readers to it after replaying what is stored.

### Outbox delivery

The run result and its events are written in one transaction. A dispatcher task
reads undelivered events and calls each adapter interested in that event kind.
Each adapter has its own retry policy:

- **webhook**: a few tries with backoff, then marked given up.
- **deploy**: the `run.succeeded` payload (repo, commit, ref, output paths)
  plus `deployment` (the file's store path and VMs), when the run is on
  `deploy.branch`. In the monolith with a control plane it is published in
  process (`ControlPlaneDeploy` in the root crate); otherwise POSTed to
  `deploy.url` (`HttpDeploy`). Retried with backoff (30 s, 2 min, 10 min,
  then every 30 min) until it succeeds, because a lost deploy means the
  cluster never converges.

Webhooks and deploys have different filters and retry policies. Webhooks
are filtered by the repo-level config's `events` list, and their URL is
resolved through `SecretStore` at delivery time. Repohub gets no events: it
reads status from the CI API.

## Logs

- Stored per check try as chunks in `RunStore`, in order.
- `GET /api/runs/{id}/checks/{attr}/log` returns the stored log.
  `?follow=true` returns Server-Sent Events: stored chunks first, then live
  ones until the check finishes.
- Every log or run read goes through `Access`: the HTTP adapter passes the
  request's credential (bearer token or session cookie, opaque to the
  application), `Access` turns it into a principal and checks it against the
  repo.
- While a run is building, `GET /api/runs/{id}/log?follow=true` streams the
  run-level view, including dependency builds.
- A retention task deletes logs older than the repo's `logRetentionDays`. Run
  and check rows stay, so history and flakiness stats survive.

## Cache

- Public repo → public cache. Private repo → the owner's private cache
  (`private-<owner>`). `ci.cache` can name another cache.
- Downloads: the CI host's Nix config lists the caches as substituters with
  their public keys. Private caches are read with the service token through a
  netrc file only the service and the daemon can read.
- Uploads: `nix copy --to` with the same netrc. The token's grants: write on
  every cache it serves, read on the private ones.
- Workers that deploy from a private cache need their own read grant. That is
  outside this service.

## Security

Evaluating a flake runs the pusher's code. These apply to every Nix call the
service makes:

| Setting | Why |
|---|---|
| no `accept-flake-config` | a flake's `nixConfig` could otherwise disable the sandbox, add substituters or set a `post-build-hook` |
| CI user not in `trusted-users` | trusted users can override sandbox settings |
| `allow-import-from-derivation = false` | no builds during evaluation |
| `restrict-eval = true` plus `allowed-uris` (repohub, GitHub, cache) | no arbitrary fetches during evaluation |
| sandbox on (NixOS default) | builds see neither host files nor the service's token |
| netrc readable by the service user only, never in a build environment | the token stays out of builds |
| repo path resolved by the service. `POST /api/runs` needs the hook's service token. | no evaluating arbitrary host paths, no runs triggered by strangers |

Remaining gaps until the VM step: fixed-output derivations have network access,
and builds share the host kernel and store. The end goal is one ephemeral
cloud-hypervisor VM per run (the `Builder` VM adapter), with its own store,
pulling from the cache.

## Service config (TOML)

On top of today's keys, all of which must actually be used (`domain`
currently is not):

- `max_concurrent_runs`, `eval_workers`, `allowed_uris` (done in step 2)
- limits: `max_reruns` (replaces `max_retries`), `max_check_timeout`,
  `max_run_timeout` (done in step 3), `max_log_retention_days` (done in
  step 6)
- `default_log_retention_days` (done in step 6)
- `cache_base_url`, `public_cache`, `private_cache_prefix`, `netrc_file`
  (done in step 4)
- `secrets_file` (done in step 5)
- `hook_token_hash`: the token the post-receive hook sends (not done)
- split deployment only: `access` adapter selection (`repohub-api` with a
  base URL, or `jwt` with a public key file or key-set URL). In the monolith
  the composition root wires access in code.

## HTTP API

| Method | Path | Notes |
|---|---|---|
| POST | `/api/runs` | from the hook, with the hook token. Returns 202 plus the run id. |
| GET | `/api/runs/{id}/log` | run-level log including dependency builds, SSE with `follow=true` |
| GET | `/api/repos/{owner}/{name}/runs` | paginated |
| GET | `/api/runs/{id}` | run with check summary |
| GET | `/api/runs/{id}/checks` | checks with status and tries |
| GET | `/api/runs/{id}/checks/{attr}/log` | stored, or SSE with `follow=true` |
| GET | `/health` | |

`builds` is renamed to `runs`. The post-receive hook (one copy only, in
`nix/modules/repohub/`) is updated to send `repo` as `owner/name` and no path.

## Module layout

Follows `.claude/CLAUDE.md`: everything private unless exported, `mod.rs` only
re-exports, no `type` aliases, no needless clones.

```
ci_service/src/
  lib.rs, main.rs, config.rs
  domain/        mod.rs, run.rs, check.rs, repo_config.rs, event.rs
  application/   mod.rs, ports.rs, error.rs, service.rs,
                 service/{enqueue, pipeline, logs, outbox, retention}.rs
  adapters/      mod.rs, sqlite.rs, memory.rs, git.rs, secrets.rs, system.rs,
                 nix/{builder, config_source, internal_json}.rs,
                 http/{router, runs, logs}.rs,
                 notify/{webhook, deploy_http}.rs,
                 access/{repohub_api, jwt}.rs     (split deployment only)

src/  (root `procurator` crate: the monolith's composition root)
  main.rs        builds every crate's Service and mounts their routers
  wiring/        in-process adapters, e.g. RepohubAccess (impl ci_service Access),
                 ControlPlaneDeploy (impl ci_service Notifier)
```

## Build order

1. **Skeleton.** ✅ Done 2026-11-14. Domain types, ports, in-memory adapters,
   `RunStore` on SQLite with the new schema, claim with `RETURNING` and
   stale-run recovery. Also the run API and the hook's new payload. `Check`
   types and store methods were left for step 2, where they are first used.
2. **Evaluate and build.** ✅ Done 2026-11-15. `nix-eval-jobs` plus one
   `nix build`, internal-json parser mapping lines to derivations, per-check
   status (with the machine that built it) and stored logs, security
   options, buildable systems from `nix config show` (host + `builders`),
   worker loop with `max_concurrent_runs`, NixOS module PATH, `git` group
   and `safe.directory`. Uses `git+file://…?rev=`. `ci_service` no longer
   uses `repo_outils`, so the lost stderr in `repo_outils::nix::run_command`
   no longer matters here (`flake_check` has no callers left).
3. **Repo config.** ✅ Done 2026-11-16. `procurator.nix` read with git from
   the checked commit, evaluated alone (`nix eval --expr`, pure, restricted,
   no URIs, 30 s, 64 KiB), validated in full (unknown keys and webhook event
   names are errors), cached in memory by blob id. Per-run settings drive the
   run: `ci.checks` (explicit lists are evaluated as indexed attributes, so a
   missing one fails alone), `ci.reruns` (failed checks only, each try
   counted in `tries`), `ci.timeouts` (`--timeout`/`--max-silent-time` per
   derivation, attributed by elapsed time because Nix's message names no
   derivation; the run limit kills the build and marks open checks
   `timed_out`), all capped by `max_reruns`, `max_check_timeout`,
   `max_run_timeout`. Repo-level keys are validated but not read yet:
   resolving them from the default branch head lands with their first user
   (cache in step 4, webhooks in step 5, retention in step 6, deploy in
   step 8). Timed-out checks are not rerun. If the builder breaks mid-run,
   open checks are closed as `failed` and the run is `errored`.
4. **Cache.** ✅ Done 2026-11-16. After building, the outputs of every
   derivation that started building in the run (dependencies and remote
   builds included) that exist, plus the paths they reference that this host
   created (`ultimate`), go up with `nix copy --no-recursive` and the netrc
   (`cache_base_url`, `netrc_file`). Downloaded paths are never re-uploaded.
   Push failures are a run-log warning, never a failed run. With no
   visibility model yet, the target is the owner's private cache
   (`<private_cache_prefix><owner>`) unless `ci.cache` on the default branch
   head (`HEAD` of the bare repo) names the public cache or that same private
   cache; any other name is refused with a warning. The NixOS module gets
   `cache.{url,publicCache,privateCachePrefix,netrcFile}` and, when
   `cache.substitute`, adds the public cache plus `cache.readCaches` to the
   host's substituters with `cache.trustedPublicKeys`. Visibility-based
   choice waits for the `Access` port (step 6).
5. **Events.** ✅ Done 2026-11-16. Outbox table `deliveries` (one row per
   webhook × wanted event, payload as JSON), written with the run's final
   status in one transaction (`finish_with_outbox`); `run.queued` and
   `run.started` are queued when they happen. Subscriptions come from
   `ci.webhooks` on the default branch head, whose ref (`HEAD` of the bare
   repo) also decides `run.recovered` (success after a failed or timed-out
   run on that branch; errored runs are skipped). One dispatcher task
   attempts due deliveries; the webhook adapter POSTs the JSON (10 s
   timeout, 2xx = delivered) and backs off 30 s, 2 min, 10 min, 30 min, then
   gives up. URLs come from `secrets_file` (TOML `name = "value"`, read at
   each delivery). HTTP client: `reqwest` with rustls (feature `webhook`).
   No events table beyond the outbox yet; repohub still reads the CI API.
6. **Monolith, access and live logs.** ✅ Done 2026-11-16.
   - **Access:** `Access` port (`authenticate` a `Credential` into a
     `Principal`, `can_read` a repo). Every read use case (run, runs,
     checks, logs, follows) takes the caller's credential and checks it in
     the application layer; 401 for an invalid credential, 403 without
     access. The HTTP adapter reads `Authorization: Bearer`. The only
     adapter is `OpenAccess` (everyone reads everything), used by both
     binaries: repohub has no credentials or permissions to wrap yet, so
     `RepohubAccess` waits for them (open question 2). Enqueueing is not
     checked yet (`hook_token_hash` still to do).
   - **Live logs:** every line gets a sequence number from the store. The
     application keeps one broadcast channel per followed run
     (`service/live.rs`); a reader subscribes, then replays the store, and
     skips what it already has, so nothing is lost or repeated; a reader
     that falls behind reloads from the store. `?follow=true` on both log
     endpoints streams SSE (`data` is `{"seq","origin","text"}`, `id` the
     seq) and ends with an `end` event when the run ends (a check's stream
     too, since a dependency can still join it). Following a check before
     the run is evaluated is 404.
   - **Dependency logs:** before building, the builder lists each check's
     derivation closure (`nix-store --query --requisites`); when a
     derivation in it starts building, it is linked to the check
     (`check_derivations`), before its first line. A check's log is its
     own lines, its derivation's and its linked dependencies', labelled
     `name> `.
   - **Retention:** a finished run's logs expire after `ci.logRetentionDays`
     (default branch), else `default_log_retention_days` (30), capped by
     `max_log_retention_days` (365). The expiry is written with the final
     status; an hourly task deletes expired lines and links. Runs and
     checks stay.
   - **Monolith:** the root `procurator` crate runs `ci_service::runtime`
     (the production wiring, shared with the standalone binary), repohub and
     the cache in one process, each on the address of its own config
     (`procurator --config` names the three files), with one shutdown.
     Not yet: a NixOS module and a flake package for it.
7. **Remote builders.** ✅ Done 2026-11-16. `services.procurator.ci.remoteBuilders`
   (host, systems, SSH user and key, host key, protocol, jobs, speed,
   features) fills `nix.buildMachines` and turns on `distributedBuilds` and
   `builders-use-substitutes`. `services.procurator.ciBuilder` sets up a
   NixOS builder: a system user that is a Nix trusted user (needed to
   receive the CI host's unsigned inputs), its authorized keys restricted
   to `nix-daemon --stdio` (so ssh-ng only), sshd, and the caches as
   substituters. The CI's system discovery is tested against the exact
   `/etc/nix/machines` line NixOS writes. Not tested: a real build over
   SSH (needs a two-machine VM test).
8. **Deploy.** ✅ Done 2026-11-16. `deploy.url` and `deploy.branch` are read
   from the default branch. A run on the deploy branch also builds the
   flake's `procuratorDeployment` (see "Deploy"); when it passes, the file
   is read (fetched from a substituter if cached elsewhere), validated, and
   queued with the run's result as a `Deploy` delivery. The control plane
   crate gained `control_plane::start` (node plus Cap'n Proto server on
   their own thread, port bound before it returns) and a `Publisher`; its
   node numbers the generation and skips an intent that is already active.
   The monolith runs it when its config names a `control_plane` file and
   deploys through `ControlPlaneDeploy`; without one, and in the standalone
   binary, `HttpDeploy` POSTs to `deploy.url`. `libs.<system>.deployment`
   writes the file from `diskVm` systems. Tested end to end with the real
   monolith (generation 1 published, the same commit again left it at 1,
   a feature branch did not deploy). Not yet: a worker booting what was
   deployed (the worker must have the store paths; it does not fetch them),
   ordering between runs that finish out of order, and more than one repo
   deploying to one control plane (each deploy replaces the whole state).
9. **Split deployment** (when needed). Repohub-API or JWT `Access` adapter.
   (The HTTP deploy adapter landed in step 8.)
10. **Isolation.** VM `Builder` adapter.

Gaps left by steps 1–8 (plan tasks T11–T14): no NixOS VM test for AC1/AC8,
the post-receive hook posts to port 3000 (CI listens on 8082) without a
token, no flake package or NixOS module for the monolith, and the deploy
follow-ups listed under step 8.

NixOS module, part of step 2 (done): `path = [ config.nix.package pkgs.git nix-eval-jobs ]`,
read access to the git-server repos for the CI user (`reposGroup`, default
`git`), `allowedUris`, `maxConcurrentRuns`, `evalWorkers`. The evaluation
security options are passed on every `nix-eval-jobs` call by the adapter.

## Acceptance criteria

- AC1: Pushing a commit whose checks all pass ends in `run.succeeded`, with one
  `passed` check per attribute and a stored log for each. A NixOS VM test
  covers git-server, CI service and cache together.
- AC2: Pushing the same commit again, after its outputs are in the cache,
  builds nothing. Every check is `cached`, and the run takes about as long as
  evaluation.
- AC3: With one failing and one passing check, the run is `failed`, the passing
  check is still built (`--keep-going`), and `check.failed` names the right
  attribute.
- AC4: `reruns = 2` builds a failing check 3 times and records 3 tries.
  `reruns` above `max_reruns` is capped.
- AC5: A check for a system with no builder is `skipped` and does not fail the
  run. A check missing from the cache is built on a remote builder of its
  system when one is configured, otherwise on the CI host, and the check
  records which.
- AC6: A user without access to the repo gets 403 on its runs and logs.
- AC7: A webhook configured for `run.failed` receives exactly one POST per
  failed run. A webhook URL that is down is retried, then given up, without
  blocking other adapters.
- AC8: A flake whose `nixConfig` sets `sandbox = false`, or which uses
  import-from-derivation, cannot change settings or build during evaluation.
- AC9: The application layer is tested with in-memory adapters only, and
  `domain/` imports no `tokio`, `axum` or `sqlx`.
- AC11: The monolith runs ci_service, repohub and cache in one process. A UI
  request for a run's log reaches `ci_service::Service` by a direct call, and
  access is checked through `RepohubAccess` with no HTTP in between.
- AC13: `cargo test -p ci_service` passes without building `repohub`, `cache`
  or `control_plane`: `ci_service`'s `Cargo.toml` depends on none of them.
- AC12: A run on `deploy.branch` with `deploy.url` set delivers its
  deployment once: published to the embedded control plane in the monolith,
  POSTed to that URL otherwise. If it fails, it is retried until it
  succeeds.
- AC10: Changing only `src/` (not `procurator.nix`) between two pushes
  evaluates the config once.

## Open questions

1. ~~**`deploy.url` in the monolith.**~~ Decided 2026-11-16: keep
   `deploy.url`. In the monolith with a control plane it only means "on";
   the URL is used when deploys go over HTTP.
2. **Repohub has no credentials or permissions yet.** Its database has
   users, projects and project members, but nothing authenticates a
   request, and nothing says who may read a repo. Until it does, CI uses
   `OpenAccess` ("everyone can read everything", fine for a single-user
   homelab) and there is no `RepohubAccess` to write. Real access control
   needs those use cases added to repohub first.

## Current state (2026-11-14)

The scaffold that steps 1–5 replaced; step 8 added the deploy call. All
fixed.

- `builds.rs::git_url` produces `/path/repo.git#<sha>`, which Nix reads as an
  output attribute. Every build fails with "could not find a flake.nix file".
- The NixOS unit has no `nix` or `git` on its PATH.
- `get_pending` claims one row, then selects a possibly different `running`
  row. Stale `running` rows are never recovered.
- Failed builds are retried with no backoff, even deterministic check failures.
- `repo_outils::nix::run_command` replaces stderr with `"Build failed"`. The
  parsed summary is discarded (`BuildSummary` is a unit struct).
- `max_retries`, `worker_poll_interval_ms`, `repos_base_path` and `domain` are
  ignored.
- `bare_repo_path` comes from an unauthenticated request.
- No cache configuration, no push, no notifications, no control-plane call.
- The post-receive hook exists twice.
