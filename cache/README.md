# Cache

An artifact cache you can run as a service or embed as a library. One process
serves several named caches; each is **public** (anyone reads, a token pushes)
or **private** (a token is needed to read and to push). Today it speaks the Nix
binary-cache protocol. The storage and access layers know nothing about Nix, so
other tools (Bazel, Buck2) can be added as further front-ends.

## Walkthrough: push and pull with Nix

### 1) Start the cache

```bash
cd /home/lucas/Projects/procurator
cargo run -p cache -- --config ./cache/example.toml
```

The example config binds `127.0.0.1:8081`, stores data under `/tmp/pcache`, and
defines two caches:

| Cache | URL | Read | Push |
|---|---|---|---|
| `public` | `http://127.0.0.1:8081/public` | anyone | token with `write` |
| `private` | `http://127.0.0.1:8081/private` | token with `read` | token with `write` |

Leave it running in its own terminal.

### 2) Create a token

Every push needs a token. Tokens are minted through the admin API with an admin
token. The example config's bootstrap admin token is the literal string
`dev-admin-token`:

```bash
curl -sS -X POST http://127.0.0.1:8081/_api/v1/tokens \
  -H 'Authorization: Bearer dev-admin-token' \
  -H 'Content-Type: application/json' \
  -d '{"name":"laptop","grants":[{"cache":"private","permission":"write"},{"cache":"public","permission":"write"}]}'
```

The response contains `"token": "pcr_…"`. It is shown once; only its hash is
stored. Give it to Nix through a netrc file:

```bash
echo "machine 127.0.0.1 password pcr_…" > ~/.config/nix/netrc
chmod 600 ~/.config/nix/netrc
```

### 3) Build something and push it

```bash
cd example
nix build .
STORE_PATH=$(readlink -f ./result)
nix copy --to http://127.0.0.1:8081/private \
  --option netrc-file ~/.config/nix/netrc "$STORE_PATH"
```

The cache checks each upload's bytes against its file name, validates the
narinfo, and signs it with the cache's key. Locally built paths carry no
signature of their own, so this server-side signature is what clients verify.

### 4) Pull it back

To prove the path comes from the cache, copy it into an empty store:

```bash
nix copy --from http://127.0.0.1:8081/private --to /tmp/pulled-store \
  --option netrc-file ~/.config/nix/netrc \
  --option trusted-public-keys "$(cat cache/cache-public.pem)" \
  "$STORE_PATH"
ls /tmp/pulled-store/nix/store
```

Without the netrc file the pull fails (the cache is private). With a different
public key it fails with "lacks a signature by a trusted key".

### 5) Use it as a substituter

```nix
nix.settings.extra-substituters = [ "http://127.0.0.1:8081/public" ];
nix.settings.extra-trusted-public-keys = [ "cache.local:hfNlzq2QP6JLd9ghjD/6+IZ3A21etwghcrKP5wvJV2k=" ];
# For a private cache, also:
nix.settings.netrc-file = "/etc/nix/netrc";
```

Use the `extra-` forms so you keep cache.nixos.org. The server prints each
cache's public key at startup.

## Configuration

One TOML file, passed with `--config`. See [example.toml](example.toml).

| Key | Default | Meaning |
|---|---|---|
| `addr` | `127.0.0.1:8081` | Address to bind. Plain HTTP only. |
| `storage_dir` | `/var/lib/procurator-cache` | Holds `blobs/`, `staging/`, `index.sqlite`. |
| `admin_token_sha256` | none | Hex SHA-256 of the bootstrap admin token. |
| `gc.interval_secs` | `3600` | Seconds between GC passes. |
| `gc.orphan_grace_secs` | `86400` | How long an uploaded NAR may wait for its narinfo. |

Each `[caches.<name>]` table (at least one is required):

| Key | Default | Meaning |
|---|---|---|
| `visibility` | required | `public` or `private`. |
| `sign_key_file` | required | Key from `nix-store --generate-binary-cache-key`. |
| `kind` | `nix` | Protocol. Only `nix` exists. |
| `store_dir` | `/nix/store` | Store prefix of the paths served. |
| `priority` | `30` | Substituter priority advertised to clients. |
| `max_size` | `0` | Quota such as `50G`; `0` is unlimited. |
| `retention_days` | `0` | Days a path is kept after its last read; `0` keeps it until space is needed. |

Cache names are 1–63 characters of `[a-z0-9-]`. Invalid config stops startup.

Generate real secrets before deploying:

```bash
nix-store --generate-binary-cache-key my-cache-1 secret.pem public.pem
TOKEN=$(openssl rand -hex 32); printf %s "$TOKEN" | sha256sum   # hash goes in the config
```

## Deploying on NixOS

The flake ships a NixOS module ([../nix/modules/cache.nix](../nix/modules/cache.nix))
that generates the TOML config and runs the server as a hardened systemd unit
(`procurator-cache.service`) under its own user.

### 1) Add the module and the overlay

The overlay provides `pkgs.cache`, which the module uses as its default package.

```nix
{
  inputs.procurator.url = "github:<owner>/procurator";

  outputs = { nixpkgs, procurator, ... }: {
    nixosConfigurations.cache-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        procurator.nixosModules.x86_64-linux.procurator
        { nixpkgs.overlays = [ procurator.overlays.default ]; }
        ./cache.nix
      ];
    };
  };
}
```

### 2) Create the secrets on the host

```bash
# One signing key per cache. Keep the secret half off the Nix store.
nix-store --generate-binary-cache-key cache.example.org-public-1 public.secret public.pub
nix-store --generate-binary-cache-key cache.example.org-internal-1 internal.secret internal.pub
install -D -m 0400 -o procurator-cache public.secret   /etc/procurator-cache/public.secret
install -D -m 0400 -o procurator-cache internal.secret /etc/procurator-cache/internal.secret

# Bootstrap admin token: keep the token, put only its hash in the config.
TOKEN=$(openssl rand -hex 32); echo "$TOKEN"
printf %s "$TOKEN" | sha256sum
```

The `procurator-cache` user exists once the module has been deployed; until
then, create the files as root and `chown` them after the first switch (the
service restarts on failure, so it picks them up). Use a secrets manager
(sops-nix, agenix) in place of `install` in real deployments. Pass `signKeyFile` as a **string** path: a Nix path literal
(`./public.secret`) would copy the key into the world-readable Nix store.

### 3) Configure the caches

```nix
# cache.nix
{
  services.procurator.cache = {
    enable = true;
    # Bind locally; a TLS reverse proxy is the only public entry point.
    addr = "127.0.0.1:8081";
    adminTokenSha256 = "<output of sha256sum above>";

    caches.public = {
      visibility = "public";
      signKeyFile = "/etc/procurator-cache/public.secret";
      maxSize = "100G";
    };
    caches.internal = {
      visibility = "private";
      signKeyFile = "/etc/procurator-cache/internal.secret";
      maxSize = "200G";
      retentionDays = 90;
    };
  };

  services.nginx = {
    enable = true;
    virtualHosts."cache.example.org" = {
      enableACME = true;
      forceSSL = true;
      locations."/" = {
        proxyPass = "http://127.0.0.1:8081";
        # Why: NARs can be gigabytes; stream them instead of buffering.
        extraConfig = ''
          client_max_body_size 0;
          proxy_request_buffering off;
          proxy_buffering off;
        '';
      };
    };
  };
  networking.firewall.allowedTCPPorts = [ 80 443 ];
}
```

| Option | Default | Maps to |
|---|---|---|
| `enable` | `false` | Runs `procurator-cache.service`. |
| `package` | `pkgs.cache` | The server binary. |
| `addr` | `0.0.0.0:8081` | `addr` |
| `storageDir` | `/var/lib/procurator-cache` | `storage_dir` (the only writable path) |
| `adminTokenSha256` | `null` | `admin_token_sha256` (must be 64 lowercase hex characters) |
| `gc.intervalSecs`, `gc.orphanGraceSecs` | `3600`, `86400` | `[gc]` |
| `caches.<name>.{visibility, signKeyFile, kind, storeDir, priority, maxSize, retentionDays}` | see [Configuration](#configuration) | `[caches.<name>]` |
| `openFirewall` | `false` | Opens the port from `addr`. |
| `user`, `group` | `procurator-cache` | Service account. |
| `extraArgs` | `[]` | Extra command-line arguments. |

Evaluation fails if no cache is defined or the admin hash is malformed.

### 4) Mint tokens and point clients at it

```bash
curl -sS -X POST https://cache.example.org/_api/v1/tokens \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"ci","grants":[{"cache":"internal","permission":"write"},{"cache":"public","permission":"write"}]}'
```

On client machines:

```nix
nix.settings = {
  extra-substituters = [ "https://cache.example.org/public" "https://cache.example.org/internal" ];
  extra-trusted-public-keys = [ "<contents of public.pub>" "<contents of internal.pub>" ];
  # Needed for the private cache: "machine cache.example.org password pcr_…"
  netrc-file = "/etc/nix/netrc";
};
```

Check the service with `systemctl status procurator-cache` and
`curl https://cache.example.org/readyz`.

## Access model

- A request carries a token as `Authorization: Bearer <token>` or as the
  password of HTTP Basic auth (what a netrc entry sends).
- A token has per-cache grants, `read` or `write` (`write` includes `read`),
  or is an **admin**: full access to every cache plus the token API.
- No credentials on a private cache gets `401`; a valid token without the
  grant gets `403`; an unknown or revoked token always gets `401`.

Token API (admin only):

| Request | Effect |
|---|---|
| `POST /_api/v1/tokens` with `{"name", "admin"?, "grants"?: [{"cache","permission"}]}` | Mint a token; the secret is in the response once. |
| `GET /_api/v1/tokens` | List tokens (never secrets). |
| `DELETE /_api/v1/tokens/<id>` | Revoke immediately. |

Other endpoints: `GET /healthz` (liveness), `GET /readyz` (index reachable),
`GET /metrics` (Prometheus text, admin only: bytes, quota, NAR and path counts
per cache).

## Storage and garbage collection

Uploaded files go into a content-addressed blob store keyed by SHA-256; a
SQLite index records which cache holds which object. A file pushed to two
caches is stored once, but each cache only sees what was pushed to it.

A cache at its quota rejects uploads with `507`. Each GC pass:

1. removes paths not read within `retention_days`;
2. if a cache is above 90% of its quota, evicts least-recently-read paths
   until it is at 75%;
3. removes NARs whose narinfo never arrived within the grace period;
4. deletes blobs no cache references any more.

## Embedding

The crate is a library in a hexagonal layout:

| Layer | Path | Contents |
|---|---|---|
| Domain | `src/domain/` | Cache names, digests, access rules, narinfo and signing. No IO. |
| Application | `src/application/` | `Service` with every use case, and the ports it needs. |
| Adapters | `src/adapters/` | HTTP router, local-disk blobs, SQLite index, in-memory blobs and index. |

Use cases take the calling `Principal` and authorize it themselves, so an
in-process caller gets the same checks as an HTTP client:

```rust
let service = Service::new(blobs, index, SystemEnv, caches, None, 86_400);
service.put_nar(&Principal::System, "internal", file_name, None, stream).await?;
```

`cache::adapters::http::router(Arc::new(service))` returns an axum `Router`
you can serve directly or nest in another application. The `http` and `sqlite`
cargo features (both on by default) gate those adapters.

## Tests

```bash
cargo test -p cache                               # unit + in-process HTTP tests
cargo test -p cache --test nix_e2e -- --ignored   # real `nix copy` push and pull
```

## Status

Implemented: named public/private Nix caches, token auth with an admin API,
server-side signing, hash-verified streaming uploads, deduplicated blob
storage, quotas, retention and eviction GC, health/readiness/metrics, graceful
shutdown, and a NixOS module ([../nix/modules/cache.nix](../nix/modules/cache.nix)).

Not implemented:

- **TLS.** The service speaks plain HTTP. Put a reverse proxy in front of it
  for anything beyond localhost: tokens travel in request headers.
- **Remote blob storage** (S3 and similar). Only local disk exists; the
  `BlobStore` port is where another backend goes.
- **Other protocols.** Bazel's HTTP cache and the REAPI gRPC API (Buck2) are
  not started.
- **Token expiry, and rotating the bootstrap admin token without a restart.**
- **Request idle timeouts.** A stalled upload holds its connection open.
- **Closure-aware GC.** Eviction is per path; it can remove a dependency while
  keeping a path that needs it, which makes that path a cache miss.
- **Multiple replicas.** One process owns the storage directory.

Known hazards:

- `cache-private.pem` and the `dev-admin-token` hash in `example.toml` are
  committed to this repository. Never use either outside local development.
- Nix clients cache narinfo lookups per substituter URL in
  `~/.cache/nix/binary-cache-v*.sqlite`. After repointing a URL at a different
  cache, delete that file if lookups look stale.
