"""
Procurator test workload.

Runs inside a Cloud Hypervisor VM. Performs real work to exercise the VM,
writes structured results to $RESULTS_DIR/result.json, and prints
delimited markers to stdout so the host can parse results from the serial log.
"""

import hashlib
import json
import os
import platform
import sys
import time


def step_file_io(results: dict) -> None:
    """Write and read files to exercise disk I/O."""
    test_data = "Hello from procurator VM!\n" * 100
    path = "/tmp/workload-test-file.txt"

    with open(path, "w") as f:
        f.write(test_data)

    with open(path, "r") as f:
        content = f.read()

    assert content == test_data, "File round-trip mismatch"
    results["steps"].append({
        "name": "file_io",
        "status": "pass",
        "detail": f"Wrote and read {len(test_data)} bytes",
    })


def step_compute(results: dict) -> None:
    """Do CPU work to verify compute works."""
    data = b"procurator-vm-test"
    for i in range(10_000):
        data = hashlib.sha256(data).digest()
    final_hash = data.hex()

    results["steps"].append({
        "name": "compute",
        "status": "pass",
        "detail": f"10000 SHA-256 rounds, final={final_hash[:16]}...",
    })


def step_system_info(results: dict) -> None:
    """Collect system information to verify VM environment."""
    info = {
        "hostname": platform.node(),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "cpu_count": os.cpu_count(),
    }
    results["system"] = info
    results["steps"].append({
        "name": "system_info",
        "status": "pass",
        "detail": json.dumps(info),
    })


def main() -> None:
    start = time.time()
    results = {
        "status": "running",
        "steps": [],
        "errors": [],
        "start_time": start,
    }

    steps = [step_system_info, step_file_io, step_compute]

    for step_fn in steps:
        try:
            step_fn(results)
        except Exception as e:
            results["errors"].append({
                "step": step_fn.__name__,
                "error": str(e),
            })
            results["steps"].append({
                "name": step_fn.__name__,
                "status": "fail",
                "detail": str(e),
            })

    elapsed = time.time() - start
    results["elapsed_seconds"] = round(elapsed, 3)

    # Determine overall status
    failed = [s for s in results["steps"] if s["status"] != "pass"]
    results["status"] = "fail" if failed else "pass"
    results["summary"] = (
        f"{len(results['steps'])} steps, "
        f"{len(results['steps']) - len(failed)} passed, "
        f"{len(failed)} failed"
    )

    # Write structured result to RESULTS_DIR (on the writable disk)
    results_dir = os.environ.get("RESULTS_DIR", "/var/lib/vm-results")
    os.makedirs(results_dir, exist_ok=True)
    result_path = os.path.join(results_dir, "result.json")
    with open(result_path, "w") as f:
        json.dump(results, f, indent=2)

    # Print delimited result to stdout → serial log for host parsing
    result_json = json.dumps(results)
    print(f"\n---PCR_RESULT_START---\n{result_json}\n---PCR_RESULT_END---\n",
          flush=True)

    # Exit with appropriate code
    if results["status"] == "pass":
        print(f"RESULT:PASS ({results['summary']}) in {elapsed:.1f}s")
        sys.exit(0)
    else:
        print(f"RESULT:FAIL ({results['summary']}) in {elapsed:.1f}s")
        sys.exit(1)


if __name__ == "__main__":
    main()
