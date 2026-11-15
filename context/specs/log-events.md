# Log events convention (`log.schema = 1`)

Services write their logs with plain `tracing` macros. The read-only `console` service builds its whole view by parsing these logs. Nothing is pushed anywhere: each service keeps its own logs (stdout/journald or a file), and the console reads them where they live.

This document is the contract. A line that doesn't follow it is shown as "unparsed" on the console's Sources page.

## Line format

- One JSON object per line, written by `tracing_subscriber::fmt().json().flatten_event(true)`. Fields sit at the top level next to `timestamp`, `level`, `target` and `message`.
- Convention lines carry `log.schema = 1` and a `log.event` discriminator: `msg`, `state` or `change`.
- Other lines are plain logs. The console uses them for the detail pages when they carry a scope field (`worker`, `vm`, `unit`).
- Convention lines are written at `info`, so default filters keep them.

## Endpoints

Endpoints are strings used in `msg.from` and `msg.to`:

| Endpoint | Meaning |
|---|---|
| `self` | The service that wrote the line. The console resolves it from the source configuration (`service = "control_plane"` or `"worker:<id>"`). |
| `cp` | The control plane. |
| `worker:<id>` | A worker, by its configured id. |
| `client:<ip:port>` | Any other caller seen by an RPC server (CLI, CI). |
| `vmm:<vm>` | The Cloud Hypervisor process of VM `<vm>`. |

## Message lines: `log.event = "msg"`

A message line is written once per message, when the message completes:

- **By the callee** when the callee is one of our services: the control plane for `Master` calls, the worker for `Worker` calls.
- **By the caller** when the callee is not ours (Cloud Hypervisor API), or when the call failed in transport, so the callee may never have seen it.

| Field | Type | Notes |
|---|---|---|
| `msg.kind` | string | One of `publish`, `poll`, `heartbeat`, `status`, `rpc` or `vmm`. |
| `msg.method` | string | The RPC or API method, e.g. `getAssignment` or `vm.boot`. |
| `msg.from` / `msg.to` | endpoint | See above. |
| `msg.outcome` | string | `ok`, `rejected` (the callee answered with an error) or `error` (failure or transport loss). |
| `msg.reason` | string, optional | Set when the outcome is not `ok`. |
| `latency_ms` | float | Time spent handling the call (callee) or waiting for it (caller). |
| `worker` | string, optional | Worker involved, when known. |
| `vm` | string, optional | VM involved, when known. |
| `generation` | u64, optional | Generation carried or returned. |
| `count` | u64, optional | Number of VMs carried (specs, running VMs, assignment slice). |
| `unchanged` | bool, optional | `getAssignment` only: the worker was already up to date. |

Example: a worker poll, as written by the control plane:

```json
{"timestamp":"2026-11-15T10:00:00.120Z","level":"INFO","target":"control_plane::msglog","message":"msg","log.schema":1,"log.event":"msg","msg.kind":"poll","msg.method":"getAssignment","msg.from":"worker:worker-1","msg.to":"self","msg.outcome":"ok","latency_ms":0.21,"worker":"worker-1","generation":3,"count":2,"unchanged":true}
```

Example: a Cloud Hypervisor call, as written by the worker:

```json
{"timestamp":"2026-11-15T10:00:01.002Z","level":"INFO","target":"worker::msglog","message":"msg","log.schema":1,"log.event":"msg","msg.kind":"vmm","msg.method":"vm.boot","msg.from":"self","msg.to":"vmm:web-1","msg.outcome":"ok","latency_ms":41.7,"vm":"web-1"}
```

## State lines: `log.event = "state"`

State lines are written periodically and on change, so the console can rebuild the current state from a short window of recent logs. The `scope` field says what the line describes.

### Control plane

`scope = "cluster"`: one line per state dump.

| Field | Type | Notes |
|---|---|---|
| `generation` | u64 | 0 when nothing is published. |
| `commit` | string | Empty when nothing is published. |
| `count` | u64 | Number of declared workers. |

`scope = "desired"`: one line per declared worker, written right after the cluster line.

| Field | Type | Notes |
|---|---|---|
| `worker` | string | Worker id. |
| `vms` | string | Comma-separated desired VM keys (empty when none). |
| `count` | u64 | Number of desired VMs. |

These lines are written after every publish and every `state_log_interval_millis` (default 10000).

```json
{"timestamp":"…","level":"INFO","target":"control_plane::node","message":"state","log.schema":1,"log.event":"state","scope":"cluster","generation":3,"commit":"9f2c1ab","count":2}
{"timestamp":"…","level":"INFO","target":"control_plane::node","message":"state","log.schema":1,"log.event":"state","scope":"desired","worker":"worker-1","vms":"db-1,web-1","count":2}
```

### Worker

`scope = "worker"`: one line per heartbeat.

| Field | Type | Notes |
|---|---|---|
| `worker` | string | Worker id. |
| `generation` | u64 | Last applied generation (0 before the first poll). |
| `count` | u64 | Number of VMs in the registry. |
| `host.mem_total` / `host.mem_available` | u64 | Bytes, from `/proc/meminfo`. |
| `host.load1` | float | One-minute load average. |
| `host.cpus` | u64 | Online CPUs. |

`scope = "vm"`: one line per VM, written right after the worker line.

| Field | Type | Notes |
|---|---|---|
| `worker`, `vm` | string | |
| `status` | string | `running` or `failed`. |
| `ip` | string | Guest IP. |
| `uptime_s` | u64 | Seconds since the VM was created. |
| `cpu_ms` | u64 | Cumulative CPU time of the VMM process (the console derives a rate). |
| `rss_bytes` | u64 | Resident memory of the VMM process. |
| `net_rx` / `net_tx` | u64 | Cumulative guest network bytes (Cloud Hypervisor counters). |
| `disk_read` / `disk_write` | u64 | Cumulative guest disk bytes (Cloud Hypervisor counters). |

```json
{"timestamp":"…","level":"INFO","target":"worker::heartbeat","message":"state","log.schema":1,"log.event":"state","scope":"worker","worker":"worker-1","generation":3,"count":1,"host.mem_total":16624349184,"host.mem_available":9123456000,"host.load1":0.42,"host.cpus":8}
{"timestamp":"…","level":"INFO","target":"worker::heartbeat","message":"state","log.schema":1,"log.event":"state","scope":"vm","worker":"worker-1","vm":"web-1","status":"running","ip":"10.0.0.2","uptime_s":312,"cpu_ms":10450,"rss_bytes":523112448,"net_rx":120034,"net_tx":88012,"disk_read":51200000,"disk_write":2048000}
```

## Change lines: `log.event = "change"`

| `change` | Written by | Extra fields |
|---|---|---|
| `generation` | control plane, on an accepted publish | `generation`, `commit`, `count` |
| `vm_created` | worker supervisor, after the VM booted | `vm` |
| `vm_start_failed` | worker supervisor | `vm`, `msg.reason` |
| `vm_deleted` | worker supervisor | `vm` |
| `reconciled` | worker poll loop, after applying a generation | `generation`, `created`, `deleted` |

```json
{"timestamp":"…","level":"INFO","target":"control_plane::node","message":"change","log.schema":1,"log.event":"change","change":"generation","generation":4,"commit":"a1b2c3d","count":3}
```

## Versioning

Adding optional fields keeps `log.schema = 1`. Renaming or removing a field, or changing its meaning, bumps `log.schema`. The console counts lines with an unknown schema as unparsed.
