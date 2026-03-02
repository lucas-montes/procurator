# Python VM Example: Build Artifacts → Create VM on Worker → Delete VM

This is a step-by-step tutorial for the current repository layout.

Quick path (single command once worker is running):

```nushell
nix run ./nix/examples/python-workload#worker-e2e
```

You will:

1. Build artifacts from `nix/examples/python-workload`
2. Start `worker`
3. Verify worker connectivity
4. Create a VM with `pcr-test` using the generated spec
5. Verify the VM exists
6. Delete the VM

## Prerequisites

- Run commands from repository root
- Host has runtime dependencies for `worker` (for example `cloud-hypervisor`)

Optional (for local cargo workflows):

```nushell
nix develop
```

## One-shot workflow (new)

After starting the worker, you can run the full flow (build spec, create VM, list VMs, delete VM):

```nushell
nix run ./nix/examples/python-workload#worker-e2e
```

Optional environment variables:

```nushell
$env.PCR_WORKER_ADDR = "127.0.0.1:6000"
$env.PCR_KEEP_VM = "1"   # keep VM instead of deleting it on exit
nix run ./nix/examples/python-workload#worker-e2e
```

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
