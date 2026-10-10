# Cache — Nix Binary Cache Server (read/write, mTLS)

A full Nix binary cache: serves `/{hash}.narinfo` + `/nar/{file}`, accepts
`PUT` uploads (`nix copy --to` works), signs with ed25519 keys, optional mTLS
for writes. Shared acceleration for the pipeline: users push, CI reuses,
workers pull pre-built NARs.

## Run it (2 commands)

```bash
cargo run -p cache -- --config ./example.toml
curl http://127.0.0.1:8081/nix-cache-info
```

Config is a TOML file, passed as `--config` (no env vars, no defaults hunt —
see `example.toml`). It serves your local `/nix/store` on `:8081`.

## Keys: do I need them?

No, for reads of upstream-signed paths — the cache re-serves their existing
`Sig:` lines. Yes, if you push your own builds or want clients to trust the
cache itself:

```bash
nix-store --generate-binary-cache-key cache.local-1 ./secret ./pub
```

```toml
# cache.toml
storage_dir = "/tmp/pcache"
sign_key_files = ["./secret"]
```

The server then adds `Sig: cache.local-1:...` to uploads lacking one. Clients
trust it via the `./pub` line (below).

## Use it

Point Nix at it and trust the key:

```nix
nix.settings.substituters = [ "http://127.0.0.1:8081" ];
nix.settings.trusted-public-keys = [ "cache.local-1:<base64-from-./pub>" ];
```

```bash
nix copy --to http://127.0.0.1:8081 /nix/store/<hash>-myapp    # push
nix copy --from http://127.0.0.1:8081 /nix/store/<hash>-myapp  # pull
```

Per-project instead of global: put the same two settings under
`extra-substituters` / `extra-trusted-public-keys` in the project's
`flake.nix` `nixConfig` (`pcr push` reads the first URL from there).
Note flake settings are untrusted by default — pass `--accept-flake-config`
or set `nix.settings.accept-flake-config = true`.

## NixOS module service

```nix
imports = [ inputs.procurator.nixosModules.procurator ];

services.procurator.cache = {
  enable = true;
  secretKeyFiles = [ "/run/secrets/cache.secret" ];
  openFirewall = true;
  # mTLS: writes require client certs from this CA, limited to these CNs
  tls.certFile = "/run/secrets/cache-cert.pem";
  tls.keyFile = "/run/secrets/cache-key.pem";
  tls.clientCaFile = "/run/secrets/cache-client-ca.pem";
  tls.writerAllowlist = [ "ci-writer" "pcr-push" ];
};
```

```bash
sudo nixos-rebuild switch
systemctl status procurator-cache
```

Full options (`addr`, `storeDir`, `storageDir`, `maxSize`, `priority`,
`tls.*`, `access.*`): `nix/modules/cache.nix` — it generates the TOML and
passes `--config`, so the service takes no env vars. With mTLS on, Nix clients
pass certs per substituter (Nix >= 2.34):

```bash
nix copy --to 'https://cache-host:8443?tls-certificate=/run/pcr/client.crt&tls-private-key=/run/pcr/client.key' /nix/store/...
```

## Test it manually

```bash
STORE_PATH=$(ls -d /nix/store/*-openssl-* | grep -v '\.drv$' | head -1)
HASH=$(basename "$STORE_PATH" | cut -d- -f1)
curl "http://127.0.0.1:8081/$HASH.narinfo"   # metadata + Sig lines
NARFILE=$(curl -s "http://127.0.0.1:8081/$HASH.narinfo" | awk '/^URL: /{print $2}')
curl "http://127.0.0.1:8081/$NARFILE" -o /tmp/t.nar
curl -X POST http://127.0.0.1:8081/ -H 'Content-Type: application/json' \
  -d '{"storePaths":["'$STORE_PATH'"]}'     # mass query
curl -s "http://127.0.0.1:8081/$HASH.narinfo" -o /tmp/h.narinfo
curl -X PUT --data-binary @/tmp/h.narinfo "http://127.0.0.1:8081/$HASH.narinfo"
```

## Endpoints

`GET/HEAD /{hash}.narinfo`, `PUT /{hash}.narinfo`, `GET/HEAD/PUT /nar/{file}`,
`POST /` (mass query), `GET /nix-cache-info`, `GET /healthz`, `GET /log/{path}`.
Reads are public by default; writes need a verified client cert when mTLS is
on, plus allowlist match (CN or `sha256:<fingerprint>`).

## Config file reference (`--config <path.toml>`)

```toml
addr = "0.0.0.0:8081"          # bind
store_dir = "/nix/store"       # proxied on miss
storage_dir = "/var/lib/procurator-cache"  # uploads: *.narinfo, nar/
max_size = "50G"               # soft quota (0 = unlimited)
priority = 30                  # advertised substituter priority
sign_key_files = []            # ed25519 key files (paths only, never inline)

[tls]
provider = "file"
# cert_file / key_file         # unset = plain HTTP (dev only)
# client_ca_file               # set = mTLS client certs required
# writer_allowlist = ["ci-writer"]  # CN or sha256:<hex>; empty = any verified

[access]                       # same options on every deployment
reads_require_auth = false     # true = private reads
writes_require_auth = "auto"   # auto | always | never (never + public bind = startup error)
loopback_bypass = true         # 127.0.0.1/::1 need no cert (co-located CI)
```

## Troubleshooting

- **No `--config`** — usage error, exit 2. The binary takes no env vars.
- **Unknown TOML key** — parse error naming file/line (typo guard).
- **Refusing insecure config** — open writes on a public bind; set
  `writes_require_auth` or bind loopback.
- **Permission denied on `/var/lib/...`** — point `storage_dir` at a
  writable dir for local runs.
- **`404` on a hash** — hash = first 32 chars of the basename
  (`basename /nix/store/... | cut -d- -f1`).
- **`400` on PUT** — `StorePath` hash must match the URL hash; needs a `Sig:`
  (push signed or configure server keys).
- **`401/403` on PUT / handshake failure** — auth required: missing cert,
  wrong CN, or loopback bypass off.
- **`copying 0 paths`** — the cache already serves that path; nothing to upload.

## References

- Nix binary-cache + `http-binary-cache-store` (`tls-certificate`,
  `want-mass-query`): https://nix.dev/manual/nix/2.34/store/types/http-binary-cache-store
- `nix-serve` setup tutorial: https://nix.dev/tutorials/nixos/binary-cache-setup.html
- Similar systems: [harmonia](https://github.com/nix-community/harmonia),
  [Attic](https://github.com/zhaofengli/attic),
  [niks3](https://github.com/Mic92/niks3),
  [Cachix](https://www.cachix.org),
  [cf-contrib/nix-cache](https://github.com/cf-contrib/nix-cache)
