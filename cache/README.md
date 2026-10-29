# Cache — simple upload and reuse example

This is the smallest useful workflow: upload a Nix store path to the cache once,
then reuse it later from the same cache.

## 1) Start the cache

```bash
cd /home/lucas/Projects/procurator
cargo run -p cache -- --config ./cache/example.toml
```

The example config binds to `127.0.0.1:8081` and uses `/nix/store` as the
source for missing paths.

## 2) Pick a store path to upload

```bash
STORE_PATH=$(ls -d /nix/store/*-openssl-* 2>/dev/null | grep -v '\.drv$' | head -1)
```

If nothing is returned, pick any other installed store path from `/nix/store`.

## 3) Upload it once

```bash
nix copy --to http://127.0.0.1:8081 "$STORE_PATH"
```

The cache stores the narinfo and nar payload, so the item is available for later
reuse without rebuilding or re-fetching from upstream.

## 4) Reuse it later

```bash
nix copy --from http://127.0.0.1:8081 "$STORE_PATH"
```

This is the same pattern you use for CI reuse: push once, then pull from the cache
later as needed.

## 5) Check the cache is responding

```bash
curl -fsS http://127.0.0.1:8081/nix-cache-info
curl -fsS http://127.0.0.1:8081/healthz
```

## Notes

- The cache can also serve a narinfo directly with `curl http://127.0.0.1:8081/<hash>.narinfo`.
- If you add a signing key in the config, uploaded items get a `Sig:` line automatically.
- This is the minimal local workflow; you do not need the full config reference unless you are running mTLS or custom auth.

## If you want to keep using it later

Add the cache to your Nix settings:

```nix
nix.settings.substituters = [ "http://127.0.0.1:8081" ];
```

Then Nix can pull from it automatically when the path is present in the cache.

For a full end-to-end example of building a Rust project, pushing it to the cache,
and then pulling it back, see [../example/README.md](../example/README.md).
