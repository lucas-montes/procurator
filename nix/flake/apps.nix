{
  pkgs,
  flake-utils,
  packages,
  workerLib,
}:
let
  inherit (packages) worker cli;
  inherit (workerLib) mkWorkerConfig defaults;

  workerAddr = defaults.listenAddr;

  # Local-dev TLS material. Generated on first run by the wrapper below;
  # the worker config points at the same path so they cannot drift.
  devTlsDir      = "worker/tests/data/dev-tls";
  devTlsCertPath = "${devTlsDir}/server.crt";
  devTlsKeyPath  = "${devTlsDir}/server.key";

  # Dev defaults — match `nix/lib/worker.nix` so the curl helper Just Works
  # against `nix run .#worker`. Override at runtime with environment vars
  # (see `worker-curl` below).
  devJwtSecret = defaults.proxy.jwtHs256Secret;   # "change-me"

  # Dev hostname for the proxy. The self-signed cert uses a wildcard CN
  # `*.worker.local` so subdomain-based Host headers (`<vm-id>.worker.local`)
  # pass TLS verification. We tell curl to resolve the subdomain to 127.0.0.1
  # via `--resolve`, keeping TLS verification honest without a real DNS entry.
  devProxyHost = "worker.local";
  devProxyPort = (builtins.elemAt
    (pkgs.lib.splitString ":" defaults.proxy.publicListenAddr) 1);

  mkAppWithDescription =
    drv: description:
    (flake-utils.lib.mkApp { inherit drv; })
    // {
      inherit description;
    };

  # Mints a HS256 JWT bound to `$VM_ID` and a secret read from
  # `$PCR_JWT_SECRET` (caller is responsible for both). Sets `$TOKEN`.
  # The token carries a short `exp` (10 minutes from now) because the
  # worker enforces expiry (see `worker/src/proxy/core.rs`).
  # Shared by `worker-curl` and `worker-token` so the algorithm has one
  # implementation.
  mintJwtSnippet = ''
    b64url() { ${pkgs.openssl}/bin/openssl base64 -A | tr -- '+/' '-_' | tr -d '='; }
    HEADER=$(printf '%s' '{"alg":"HS256","typ":"JWT"}' | b64url)
    # The worker accepts exactly one VM per token (`vm_id`) and requires
    # `exp`; 10 minutes is enough for ad-hoc curl/SDK usage.
    EXP=$(( $(${pkgs.coreutils}/bin/date +%s) + 600 ))
    PAYLOAD=$(printf '{"vm_id":"%s","exp":%d}' "$VM_ID" "$EXP" | b64url)
    SIG=$(printf '%s.%s' "$HEADER" "$PAYLOAD" \
          | ${pkgs.openssl}/bin/openssl dgst -sha256 -hmac "$SECRET" -binary \
          | b64url)
    TOKEN="$HEADER.$PAYLOAD.$SIG"
  '';

  configFile = pkgs.writeText "procurator-worker-config.json" (
    builtins.toJSON (mkWorkerConfig {
      vmm = {
        runtimeDir = "worker/tests/data";
        stateDir   = "worker/tests/data";
      };
      proxy = {
        baseDomain  = "worker.local";
        tlsCertPath = devTlsCertPath;
        tlsKeyPath  = devTlsKeyPath;
      };
    })
  );

  worker-wrapper = pkgs.writeShellScriptBin "procurator-worker" ''
    set -euo pipefail

    if [ ! -s "${devTlsCertPath}" ] || [ ! -s "${devTlsKeyPath}" ]; then
      echo "[dev] Generating self-signed TLS cert at ${devTlsDir}"
      mkdir -p "${devTlsDir}"
      # Wildcard cert so subdomain-based Host headers pass verification.
      # curl --resolve <vm-id>.worker.local:8443:127.0.0.1 matches the SAN.
      ${pkgs.openssl}/bin/openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout "${devTlsKeyPath}" \
        -out    "${devTlsCertPath}" \
        -days 30 -subj "/CN=*.worker.local" \
        -addext "subjectAltName=DNS:*.worker.local,DNS:worker.local" \
        2>/dev/null
    fi

    echo "[dev] sudo is required so the worker can manage TAP devices."
    exec sudo ${worker}/bin/worker ${configFile}
  '';

  worker-test-wrapper = pkgs.writeShellScriptBin "procurator-worker-test" ''
    exec ${cli}/bin/pcr-worker-test --addr ${workerAddr} "$@"
  '';

  # ── worker-token ───────────────────────────────────────────────────────
  # Print a HS256 JWT for the given VM id, bound to a single VM with a
  # 10-minute expiry. Send it via `Authorization: Bearer <token>` or via
  # the cookie-bootstrap URL: `https://<vm-id>.worker.local:8443/__pcr/auth?token=<jwt>`.
  #
  # Usage:
  #   nix run .#worker-token -- <vm-id>
  worker-token-wrapper = pkgs.writeShellScriptBin "procurator-worker-token" ''
    set -euo pipefail

    if [ "$#" -lt 1 ]; then
      cat >&2 <<'EOF'
    usage: procurator-worker-token <vm-id>

    Prints a HS256 JWT bound to <vm-id> with a 10-minute `exp`, signed
    with $PCR_JWT_SECRET. Send it as `Authorization: Bearer <token>` or
    use the cookie-bootstrap URL:
      https://<vm-id>.worker.local:8443/__pcr/auth?token=<token>

    Env vars:
      PCR_JWT_SECRET  HS256 secret  (default: dev "change-me")
    EOF
      exit 64
    fi

    VM_ID="$1"; shift

    SECRET="''${PCR_JWT_SECRET:-${devJwtSecret}}"

    ${mintJwtSnippet}

    echo "$TOKEN"
  '';

  # ── worker-curl ────────────────────────────────────────────────────────
  # Hit the worker's TLS proxy with a JWT minted for the VM id you pass.
  # Uses subdomain-based routing: https://<vm-id>.worker.local:<port>/<path>
  #
  # Usage:
  #   nix run .#worker-curl -- <vm-id> <path> [extra curl args...]
  #
  # Examples:
  #   nix run .#worker-curl -- 019e16f4-... /doc
  #   nix run .#worker-curl -- 019e16f4-... /doc -s | jq '.paths | keys'
  #   nix run .#worker-curl -- 019e16f4-... /session -X POST \
  #     -H 'content-type: application/json' -d '{}'
  #   nix run .#worker-curl -- 019e16f4-... /event -N   # SSE
  #
  # Env-var overrides (sensible defaults match `nix run .#worker`):
  #   PCR_JWT_SECRET  HS256 secret              (default: dev "change-me")
  #   PCR_PROXY_HOST  Proxy hostname            (default: worker.local)
  #   PCR_PROXY_PORT  Proxy port                (default: 8443)
  #   PCR_CACERT      CA bundle for --cacert    (default: dev self-signed cert)
  worker-curl-wrapper = pkgs.writeShellScriptBin "procurator-worker-curl" ''
    set -euo pipefail

    if [ "$#" -lt 2 ]; then
      cat >&2 <<'EOF'
    usage: procurator-worker-curl <vm-id> <path> [curl-args...]

    Mints a HS256 JWT for the given VM id and curls the worker proxy
    via subdomain-based routing (https://<vm-id>.worker.local:<port>/<path>).
    Everything after <path> is forwarded verbatim to curl.

    Env vars:
      PCR_JWT_SECRET  HS256 secret              (default: dev "change-me")
      PCR_PROXY_HOST  Proxy hostname            (default: worker.local)
      PCR_PROXY_PORT  Proxy port                (default: 8443)
      PCR_CACERT      CA bundle for --cacert    (default: dev self-signed cert)
    EOF
      exit 64
    fi

    VM_ID="$1"; shift
    REQ_PATH="$1"; shift

    SECRET="''${PCR_JWT_SECRET:-${devJwtSecret}}"
    PROXY_HOST="''${PCR_PROXY_HOST:-${devProxyHost}}"
    PROXY_PORT="''${PCR_PROXY_PORT:-${devProxyPort}}"
    CACERT="''${PCR_CACERT:-${devTlsCertPath}}"

    if [ ! -s "$CACERT" ]; then
      echo "warning: CA bundle '$CACERT' is missing or empty;" >&2
      echo "         start the worker once (nix run .#worker) to generate it," >&2
      echo "         or set PCR_CACERT to an existing file." >&2
    fi

    ${mintJwtSnippet}

    # Ensure path starts with '/'.
    case "$REQ_PATH" in
      /*) ;;
      *) REQ_PATH="/$REQ_PATH" ;;
    esac

    # Build the subdomain hostname: <vm-id>.worker.local
    SUBDOMAIN_HOST="''${VM_ID}.''${PROXY_HOST}"

    # `--resolve` forces <vm-id>.worker.local:8443 → 127.0.0.1, so TLS
    # verification uses the wildcard cert's SAN while we still hit localhost.
    exec ${pkgs.curl}/bin/curl \
      --cacert "$CACERT" \
      --resolve "''${SUBDOMAIN_HOST}:''${PROXY_PORT}:127.0.0.1" \
      -H "Authorization: Bearer $TOKEN" \
      "$@" \
      "https://''${SUBDOMAIN_HOST}:''${PROXY_PORT}''${REQ_PATH}"
  '';
  # ── worker-bootstrap-url ───────────────────────────────────────────────
  # Print a clickable browser URL that bootstraps cookie auth and redirects
  # into the VM's OpenCode console.
  #
  # Usage:
  #   nix run .#worker-bootstrap-url -- <vm-id>
  #
  # Env-var overrides:
  #   PCR_JWT_SECRET  HS256 secret              (default: dev "change-me")
  #   PCR_PROXY_HOST  Proxy hostname            (default: worker.local)
  #   PCR_PROXY_PORT  Proxy port                (default: 8443)
  worker-bootstrap-url-wrapper = pkgs.writeShellScriptBin "procurator-worker-bootstrap-url" ''
    set -euo pipefail

    if [ "$#" -lt 1 ]; then
      echo "usage: procurator-worker-bootstrap-url <vm-id>" >&2
      exit 64
    fi

    VM_ID="$1"
    SECRET="''${PCR_JWT_SECRET:-${devJwtSecret}}"
    PROXY_HOST="''${PCR_PROXY_HOST:-${devProxyHost}}"
    PROXY_PORT="''${PCR_PROXY_PORT:-${devProxyPort}}"

    ${mintJwtSnippet}

    echo "https://''${VM_ID}.''${PROXY_HOST}:''${PROXY_PORT}/__pcr/auth?token=''${TOKEN}&next=/console"
  '';
in
{
  apps = {
    worker = mkAppWithDescription worker-wrapper
      "Run the Procurator worker daemon (auto-generates dev TLS cert)";
    worker-test = mkAppWithDescription worker-test-wrapper
      "Run the test-only worker RPC CLI (read/list/create/delete)";
    worker-token = mkAppWithDescription worker-token-wrapper
      "Print a short-lived JWT bound to <vm-id> (for `Authorization: Bearer`)";
    worker-curl = mkAppWithDescription worker-curl-wrapper
      "curl the worker proxy with a JWT minted for <vm-id> <path> [curl-args...]";
    worker-bootstrap-url = mkAppWithDescription worker-bootstrap-url-wrapper
      "Print a clickable browser URL for cookie-bootstrap auth to <vm-id>";
  };
}
