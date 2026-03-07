"""
Procurator network test workload.

Runs inside a Cloud Hypervisor VM. Makes HTTP requests to test domain
allowlisting:
  - python.org  → should succeed (in allowedDomains)
  - google.com  → should be blocked (NOT in allowedDomains)

Writes structured JSON results to a file, reads them back, and prints
to stdout with delimited markers so the host can parse from serial log.
"""

import json
import os
import sys
import time
from urllib.request import urlopen
from urllib.error import URLError


ALLOWED_URL = "https://www.python.org"
BLOCKED_URL = "https://www.google.com"
TIMEOUT_SECONDS = 10


def http_get(url: str) -> dict:
    """Attempt an HTTP GET and return a result dict."""
    try:
        resp = urlopen(url, timeout=TIMEOUT_SECONDS)
        return {
            "url": url,
            "status_code": resp.status,
            "ok": True,
            "message": f"HTTP {resp.status}",
        }
    except URLError as e:
        return {
            "url": url,
            "status_code": None,
            "ok": False,
            "message": str(e.reason),
        }
    except Exception as e:
        return {
            "url": url,
            "status_code": None,
            "ok": False,
            "message": str(e),
        }


def main() -> None:
    start = time.time()

    # ── Step 1: HTTP request to allowed domain ─────────────────────
    print(f"Requesting allowed URL: {ALLOWED_URL}", flush=True)
    request_valid = http_get(ALLOWED_URL)
    print(f"  Result: {request_valid}", flush=True)

    # ── Step 2: HTTP request to blocked domain ─────────────────────
    print(f"Requesting blocked URL: {BLOCKED_URL}", flush=True)
    request_invalid = http_get(BLOCKED_URL)
    print(f"  Result: {request_invalid}", flush=True)

    # ── Step 3: Build results ──────────────────────────────────────
    results = {
        "request_valid": {
            "url": request_valid["url"],
            "status_code": request_valid["status_code"],
            "ok": request_valid["ok"],
            "message": request_valid["message"],
        },
        "request_invalid": {
            "url": request_invalid["url"],
            "status_code": request_invalid["status_code"],
            "ok": request_invalid["ok"],
            "message": request_invalid["message"],
        },
        "elapsed_seconds": round(time.time() - start, 3),
    }

    # Determine overall status:
    # - request_valid should succeed (ok=True, status_code=200-ish)
    # - request_invalid should fail (ok=False or connection refused/timed out)
    valid_passed = request_valid["ok"]
    invalid_blocked = not request_invalid["ok"]
    results["status"] = "pass" if (valid_passed and invalid_blocked) else "fail"
    results["summary"] = (
        f"allowed={'ok' if valid_passed else 'FAIL'}, "
        f"blocked={'ok' if invalid_blocked else 'FAIL (reached blocked domain!)'}"
    )

    # ── Step 4: Write results to file ──────────────────────────────
    results_dir = os.environ.get("RESULTS_DIR", "/var/lib/vm-results")
    os.makedirs(results_dir, exist_ok=True)
    result_path = os.path.join(results_dir, "result.json")

    with open(result_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"Results written to {result_path}", flush=True)

    # ── Step 5: Read results back (file I/O round-trip) ────────────
    with open(result_path, "r") as f:
        readback = json.load(f)
    print(f"Results read back from {result_path}", flush=True)

    # ── Step 6: Print delimited result to stdout → serial log ──────
    result_json = json.dumps(readback)
    print(f"\n---PCR_RESULT_START---\n{result_json}\n---PCR_RESULT_END---\n",
          flush=True)

    elapsed = time.time() - start
    if readback["status"] == "pass":
        print(f"RESULT:PASS ({readback['summary']}) in {elapsed:.1f}s")
        sys.exit(0)
    else:
        print(f"RESULT:FAIL ({readback['summary']}) in {elapsed:.1f}s")
        sys.exit(1)


if __name__ == "__main__":
    main()
