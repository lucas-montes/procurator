"""
Sandbox example workload.

Runs inside a Cloud Hypervisor sandbox VM. Tests:
  1. Python works (packages were installed correctly)
  2. Network allowlist works (allowed domain reachable, blocked not)
  3. Local files were copied correctly

Writes results to RESULTS_DIR for host-side collection.
"""

import json
import os
import sys
import time
from pathlib import Path
from urllib.request import urlopen
from urllib.error import URLError


ALLOWED_URL = "https://www.python.org"
BLOCKED_URL = "https://www.google.com"
TIMEOUT = 10


def test_http(url: str) -> dict:
    """Test HTTP connectivity to a URL."""
    try:
        resp = urlopen(url, timeout=TIMEOUT)
        return {"url": url, "ok": True, "status": resp.status}
    except URLError as e:
        return {"url": url, "ok": False, "error": str(e.reason)}
    except Exception as e:
        return {"url": url, "ok": False, "error": str(e)}


def test_local_files() -> dict:
    """Verify that local files were copied into the VM."""
    workspace = Path("/workspace")
    config = Path("/etc/sandbox/config.toml")

    return {
        "workspace_exists": workspace.is_dir(),
        "main_py_exists": (workspace / "main.py").is_file(),
        "config_exists": config.is_file(),
        "workspace_contents": sorted(str(p) for p in workspace.iterdir()) if workspace.is_dir() else [],
    }


def main() -> None:
    start = time.time()
    print("=" * 60, flush=True)
    print("Sandbox Workload Starting", flush=True)
    print("=" * 60, flush=True)

    # Test 1: Local files
    print("\n[1/3] Checking local files...", flush=True)
    files_result = test_local_files()
    print(f"  Workspace exists: {files_result['workspace_exists']}", flush=True)
    print(f"  main.py exists:   {files_result['main_py_exists']}", flush=True)
    print(f"  Config exists:    {files_result['config_exists']}", flush=True)

    # Test 2: Allowed domain
    print(f"\n[2/3] Testing allowed domain: {ALLOWED_URL}", flush=True)
    allowed_result = test_http(ALLOWED_URL)
    print(f"  Result: {allowed_result}", flush=True)

    # Test 3: Blocked domain
    print(f"\n[3/3] Testing blocked domain: {BLOCKED_URL}", flush=True)
    blocked_result = test_http(BLOCKED_URL)
    print(f"  Result: {blocked_result}", flush=True)

    # Build final results
    elapsed = round(time.time() - start, 3)
    all_pass = (
        files_result["workspace_exists"]
        and files_result["main_py_exists"]
        and allowed_result["ok"]
        and not blocked_result["ok"]
    )

    results = {
        "status": "pass" if all_pass else "fail",
        "local_files": files_result,
        "allowed_domain": allowed_result,
        "blocked_domain": blocked_result,
        "elapsed_seconds": elapsed,
    }

    # Write results
    results_dir = os.environ.get("RESULTS_DIR", "/var/lib/sandbox-results")
    os.makedirs(results_dir, exist_ok=True)
    result_path = os.path.join(results_dir, "result.json")

    with open(result_path, "w") as f:
        json.dump(results, f, indent=2)

    print(f"\nResults written to {result_path}", flush=True)
    print(f"\n{'=' * 60}", flush=True)
    print(f"OVERALL: {'PASS' if all_pass else 'FAIL'} ({elapsed}s)", flush=True)
    print(f"{'=' * 60}", flush=True)

    # Print structured result for host-side parsing
    print(f"\n---SANDBOX_RESULT_BEGIN---", flush=True)
    print(json.dumps(results), flush=True)
    print(f"---SANDBOX_RESULT_END---", flush=True)

    sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()
