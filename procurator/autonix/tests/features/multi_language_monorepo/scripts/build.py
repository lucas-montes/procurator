#!/usr/bin/env python3
"""Build automation scripts"""

import subprocess
import sys

def build_all():
    print("Building all projects...")
    # Rust backend
    subprocess.run(["cargo", "build"], cwd="../backend")
    # Node frontend
    subprocess.run(["npm", "install"], cwd="../frontend")

if __name__ == "__main__":
    build_all()
