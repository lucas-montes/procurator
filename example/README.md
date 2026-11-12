# Local cache example

This example shows a minimal Rust project that you can build locally, push to the
cache, and then reuse from that cache later.

## 1) Start the cache

From the repository root:

```bash
cargo run -p cache -- --config ./cache/example.toml
```

That config serves a `public` cache (anyone reads, a token pushes) at
`http://127.0.0.1:8081/public`, signed with the committed dev key.

Pushing needs a token. Mint one with the example config's bootstrap admin token
and hand it to Nix through a netrc file:

```bash
curl -sS -X POST http://127.0.0.1:8081/_api/v1/tokens \
  -H 'Authorization: Bearer dev-admin-token' \
  -H 'Content-Type: application/json' \
  -d '{"name":"example","grants":[{"cache":"public","permission":"write"}]}'
echo "machine 127.0.0.1 password <token from the response>" > /tmp/cache-netrc
```

## 2) Build the project

```bash
cd example
nix build .
```

Use `nix build`, not `cargo build`: only a Nix store path can be copied to the
cache. `cargo build` writes to `target/`, which the cache has no way to address.

## 3) Push the built result to the cache

```bash
nix copy --to http://127.0.0.1:8081/public \
  --option netrc-file /tmp/cache-netrc ./result
```

The cache signs what you push; locally built paths carry no signature of their
own.

## 4) Pull it back from the cache later

Reading a public cache needs no token. Copy into an empty store to prove the
path comes from the cache:

```bash
nix copy --from http://127.0.0.1:8081/public --to /tmp/pulled-store \
  --option trusted-public-keys "$(cat ../cache/cache-public.pem)" \
  "$(readlink -f ./result)"
```

The public key has to be trusted or nix rejects the path with "lacks a signature
by a trusted key".

## 5) Reuse the cache in the flake

The flake includes a `nixConfig` block so the project automatically uses the local cache:

```nix
nixConfig = {
  extra-substituters = [ "http://127.0.0.1:8081/public" ];
  extra-trusted-public-keys = [ "cache.local:hfNlzq2QP6JLd9ghjD/6+IZ3A21etwghcrKP5wvJV2k=" ];
  accept-flake-config = true;
};
```

If you prefer the non-flake form, the equivalent is:

```nix
nix.settings.extra-substituters = [ "http://127.0.0.1:8081/public" ];
nix.settings.extra-trusted-public-keys = [ "cache.local:hfNlzq2QP6JLd9ghjD/6+IZ3A21etwghcrKP5wvJV2k=" ];
```

Use the `extra-` prefixed keys. The bare `substituters` and
`trusted-public-keys` *replace* the system defaults, which drops
cache.nixos.org and makes a plain `nix build` here attempt to bootstrap stdenv
from source — around 900 derivations.

## Notes

- The public key above is the throwaway dev key committed at
  `../cache/cache-public.pem`, so this example runs as-is. Generate your own
  before trusting the cache for anything real — the matching secret key is in
  the repository:
  `nix-store --generate-binary-cache-key cache.local cache-private.pem cache-public.pem`
- This is just a minimal example; the important part is the build -> push -> reuse pattern.
