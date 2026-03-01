# Testing the Worker

`pcr-test` is a development CLI for manually testing the Worker RPC interface. It covers all four methods defined in `worker.capnp`.

Available in the nix dev shell (`nix develop`).

## Commands

### read — Worker status

```nushell
pcr-test read
pcr-test --addr 127.0.0.1:6000 read
```

### list-vms — List all VMs

```nushell
pcr-test list-vms
```

### create-vm — Create a VM

From a JSON spec file (output of `nix build .#vmSpecJson`):

```nushell
pcr-test create-vm --spec-file ./result/vm-spec.json
```

From individual flags:

```nushell
pcr-test create-vm \
  --toplevel /nix/store/...-nixos-system \
  --kernel-path /nix/store/...-linux/bzImage \
  --initrd-path /nix/store/...-initrd/initrd \
  --disk-image-path /nix/store/...-nixos-ext4.img \
  --cpu 2 \
  --memory-mb 1024 \
  --allowed-domain api.openai.com \
  --allowed-domain github.com
```

### delete-vm — Delete a VM

```nushell
pcr-test delete-vm <VM_ID>
```

## Workflow

```nushell
# 1. Start the worker (separate terminal)
cargo run -p worker

# 2. Check worker status
pcr-test read

# 3. Create a VM
pcr-test create-vm --spec-file ./vm-spec.json

# 4. Verify it's running
pcr-test list-vms

# 5. Delete it
pcr-test delete-vm <ID_FROM_STEP_3>

# 6. Confirm deletion
pcr-test list-vms
```

## Debug logging

```nushell
$env.RUST_LOG = "debug"; pcr-test read
```
