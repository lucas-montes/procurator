# Console — read-only cluster view from logs

## What

A small web server and UI that shows the cluster live:

- **Map:** machines with their VMs, and the messages between services as they happen. Polls, heartbeats, publishes, RPCs and Cloud Hypervisor calls animate along the edges, and failures show in red.
- **Machine, unit and VM pages:** status, live metrics, logs and messages.
- **Sources page:** the health of every log source.

It builds everything **only by reading the logs each service already writes**. Services don't send anything anywhere and don't know the console exists. The console performs no actions and makes no RPC calls.

## How

Services log JSON lines through plain `tracing` macros, following [`context/specs/log-events.md`](../context/specs/log-events.md):

- **Message lines:** one per RPC, poll, heartbeat or Cloud Hypervisor call.
- **State lines:** written periodically, desired state from the control plane and observed state with metrics from each worker.
- **Change lines and plain logs.**

The console follows each service's logs from one of three sources, configured per service:

| Kind | When | Config |
|---|---|---|
| `file` | Manual runs: the service logs to a file (`"log": {"file": {"path": …}}` in its config) | `{"file": {"path": "/tmp/pcr/cp.log"}}` |
| `journal_gateway` | systemd deployments: `systemd-journal-gatewayd` on each host | `{"journal_gateway": {"url": "http://10.0.0.11:19531", "unit": "procurator-worker.service"}}` |
| `journal_local` | A systemd service on the console's own host | `{"journal_local": {"unit": "procurator-control-plane.service"}}` |

At startup each source reads back its last `bootstrap_entries` lines to rebuild the current state. After that the view is **live only**: no history is kept or shown.

## Run it by hand

```sh
# Services log to files instead of stdout:
#   control plane config: "log": {"file": {"path": "/tmp/pcr/cp.log"}}
#   worker config:        "log": {"file": {"path": "/tmp/pcr/w1.log"}}
cat > /tmp/pcr/console.json <<'JSON'
{
  "http_addr": "127.0.0.1:8095",
  "sources": [
    {"name": "cp", "service": "control_plane", "kind": {"file": {"path": "/tmp/pcr/cp.log"}}},
    {"name": "worker-1", "service": "worker:worker-1", "kind": {"file": {"path": "/tmp/pcr/w1.log"}}}
  ]
}
JSON
cargo run -p console -- /tmp/pcr/console.json   # then open http://127.0.0.1:8095
```

`service` tells the console who wrote the lines, so it can resolve the `self` endpoint: `control_plane` or `worker:<id>`.

## NixOS

```nix
# On the hosts being watched:
services.procurator.journalGateway.enable = true;   # unauthenticated: trusted network only

# On the console host:
services.procurator.console = {
  enable = true;
  controlPlane = "cp-1";            # names in cluster.vms
  workers = [ "worker-1" "worker-2" ];
};
```

## HTTP surface

| Route | Content |
|---|---|
| `/`, `/machines[/<id>]`, `/units[/<unit>]`, `/vms/<worker>/<key>`, `/sources` | The UI (one HTML shell, routed in `assets/app.js`) |
| `GET /api/model` | Current snapshot (JSON) |
| `GET /api/stream` | SSE: `state` (snapshot on change), `message`, `log`, `lagged` |

There is one stream per page because browsers allow only six HTTP/1.1 connections per host.

## Status

Not done yet:

- **No authentication or TLS.** It binds to loopback by default. journal-gatewayd has no authentication either.
- **Deployment units are not published yet.** VMs are grouped by a `unit` field only if their logs carry one, so everything is "ungrouped" for now.
- **Nothing from inside the VMs** (guest services and guest logs). That is planned separately.
- **Cloud Hypervisor calls are logged with the worker's internal VM id** (`vmm:<id>`). The map draws them on the machine's VMM anchor, but the VM page doesn't list them.
