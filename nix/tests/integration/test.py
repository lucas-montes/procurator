#!/usr/bin/env python3
"""Integration test workload — runs inside the VM on boot.

Validates that the VM environment is correctly configured:
  1. Python is available (we're running)
  2. Filesystem is writable
  3. Network config is present
  4. System info is accessible
  5. Injected files are present

Writes a JSON report to /var/lib/workload/report.json.
"""

import json
import os
import socket
import subprocess
import sys
import time


def check(name, fn):
    """Run a check and return (name, passed, detail)."""
    try:
        detail = fn()
        return (name, True, detail)
    except Exception as e:
        return (name, False, str(e))


def main():
    results = []

    # 1. Python is available
    results.append(check("python_available", lambda: sys.version))

    # 2. Filesystem is writable
    def write_test():
        os.makedirs("/var/lib/workload", exist_ok=True)
        with open("/var/lib/workload/test.txt", "w") as f:
            f.write("hello from integration test\n")
        with open("/var/lib/workload/test.txt", "r") as f:
            return f.read().strip()

    results.append(check("filesystem_writable", write_test))

    # 3. Hostname is set
    results.append(check("hostname", lambda: socket.gethostname()))

    # 4. System info
    results.append(check("kernel", lambda: os.uname().release))
    results.append(check("uptime", lambda: open("/proc/uptime").read().split()[0]))

    # 5. Injected file present
    def check_injected_file():
        path = "/etc/procurator/test-marker"
        if not os.path.exists(path):
            raise FileNotFoundError(f"{path} not found")
        return open(path).read().strip()

    results.append(check("injected_file", check_injected_file))

    # 6. Nix store is registered
    def check_nix_store():
        result = subprocess.run(
            ["nix-store", "--verify", "--check-contents"],
            capture_output=True,
            text=True,
            timeout=30,
        )
        return f"exit={result.returncode}"

    results.append(check("nix_store", check_nix_store))

    # Build report
    report = {
        "workload": "integration-test",
        "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "hostname": socket.gethostname(),
        "python": sys.version,
        "checks": [
            {"name": name, "passed": passed, "detail": detail}
            for name, passed, detail in results
        ],
        "all_passed": all(passed for _, passed, _ in results),
    }

    # Write report
    os.makedirs("/var/lib/workload", exist_ok=True)
    report_path = "/var/lib/workload/report.json"
    with open(report_path, "w") as f:
        json.dump(report, f, indent=2)

    # Print to console (visible in serial output)
    print("\n=== Integration Test Report ===")
    for name, passed, detail in results:
        status = "PASS" if passed else "FAIL"
        print(f"  [{status}] {name}: {detail}")

    if report["all_passed"]:
        print("\nAll checks passed!")
    else:
        print("\nSome checks FAILED!")
        sys.exit(1)


if __name__ == "__main__":
    main()
