# Python VM Example: Build Artifacts → Create VM on Worker → Verify Results → Delete VM

This is a step-by-step tutorial for the current repository layout.

Run the worker with:
```nushell
nix run ./nix#worker
```


Quick path (single command once worker is running):

```nushell
nix run ./nix/examples/python-workload#worker-e2e
```

The e2e test:

1. Checks worker connectivity
2. Creates a VM from the python workload spec
3. Waits for the VM workload to complete (polls serial log for result markers)
4. Parses structured JSON results from serial output
5. Verifies the workload passed
6. Deletes the VM
7. Confirms VM is removed from list

## Prerequisites

- Run commands from repository root
- Host has runtime dependencies for `worker` (for example `cloud-hypervisor`)

Optional (for local cargo workflows):

```nushell
nix develop
```

## One-shot workflow

After starting the worker, run the full e2e test:

```nushell
nix run ./nix/examples/python-workload#worker-e2e
```

Optional environment variables:

```nushell
$env.PCR_WORKER_ADDR = "127.0.0.1:6000"  # default
$env.PCR_VM_DIR = "/tmp/procurator/vms"   # where worker puts VM dirs
$env.PCR_TIMEOUT = "120"                   # max seconds to wait for results
nix run ./nix/examples/python-workload#worker-e2e
```

## How results work

The VM workload (`test.py`) writes structured JSON between delimiters to stdout:

```
---PCR_RESULT_START---
{"status": "pass", "steps": [...], "summary": "3 steps, 3 passed, 0 failed"}
---PCR_RESULT_END---
```

This flows through systemd's `journal+console` → serial console → host file at
`/tmp/procurator/vms/{vm_id}/serial.log`. The e2e script polls this file and
extracts the JSON result.

The workload also writes `result.json` to `$RESULTS_DIR` (`/var/lib/vm-results/`)
on the writable disk image inside the VM.

## Step 1: Build Python example artifacts

Build the Python workload image and the example JSON spec:

```nushell
let vm_image_path = (nix build ./nix/examples/python-workload#vm-image --print-out-paths | str trim)
let worker_spec_path = (nix run ./nix/examples/python-workload#build-vm-spec-path | str trim)

print $vm_image_path
print $worker_spec_path
```

Check the worker spec content:

```nushell
open $worker_spec_path
```

## Step 2: Start the worker

Use a separate terminal and keep it running:

```nushell
nix run ./nix#worker
```

## Step 3: Verify worker connectivity

```nushell
nix run ./nix#pcr-test -- read
```

If your worker listens on a different address:

```nushell
nix run ./nix#pcr-test -- --addr 127.0.0.1:6000 read
```

## Step 4: Create the VM

```nushell
let create_output = (nix run ./nix#pcr-test -- create-vm --spec-file $worker_spec_path)
print $create_output
```

Extract VM ID from the output (if needed):

```nushell
let vm_id = ($create_output | parse --regex '([0-9a-fA-F-]{36})' | get 0.capture0)
print $vm_id
```

## Step 5: List VMs and confirm creation

```nushell
nix run ./nix#pcr-test -- list-vms
```

Confirm your `vm_id` is present in the list.

## Step 6: Delete the VM

```nushell
nix run ./nix#pcr-test -- delete-vm $vm_id
```

Confirm deletion:

```nushell
nix run ./nix#pcr-test -- list-vms
```

## Optional: Debug logging

```nushell
$env.RUST_LOG = "debug"; nix run ./nix#pcr-test -- read
```

## Troubleshooting

- If `nix run ./nix#pcr-test -- ...` fails, run from repo root so relative paths resolve.
- If `create-vm` fails, confirm `worker` is running and reachable.
- If VM boot fails after RPC succeeds, check worker logs in the worker terminal (hypervisor/runtime issue).
- If the e2e test times out waiting for results, check the serial log directly:
  ```nushell
  cat /tmp/procurator/vms/<vm-id>/serial.log | tail -50
  ```
- If `prepare()` fails with "Artifact not found", ensure you've built the VM image first:
  ```nushell
  nix build ./nix/examples/python-workload#vm-image
  ```
