#!/usr/bin/env python3
"""Reliable mock TCP server — replaces nc -lk 8080.

Listens on port 8080, accepts multiple concurrent connections,
logs received data to stdout. Handles SIGTERM/SIGINT gracefully.
"""

import os
import signal
import socket
import socketserver
import sys


class MockTCPHandler(socketserver.BaseRequestHandler):
    """Handles a single client connection."""

    def handle(self):
        addr = self.client_address
        sys.stdout.write(f"connection from {addr[0]}:{addr[1]}\n")
        sys.stdout.flush()
        try:
            while True:
                data = self.request.recv(4096)
                if not data:
                    break
                text = data.decode("utf-8", errors="replace").rstrip("\n")
                sys.stdout.write(f"{text}\n")
                sys.stdout.flush()
        except ConnectionResetError:
            sys.stdout.write(f"connection reset by {addr[0]}:{addr[1]}\n")
            sys.stdout.flush()
        except OSError:
            # Connection closed or socket error
            pass
        finally:
            sys.stdout.write(f"disconnected {addr[0]}:{addr[1]}\n")
            sys.stdout.flush()


class ThreadedTCPServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    host = sys.argv[1] if len(sys.argv) > 1 else "0.0.0.0"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8080

    server = ThreadedTCPServer((host, port), MockTCPHandler)

    # Graceful shutdown on SIGTERM / SIGINT
    shutdown_event = False

    def sighandler(signum, frame):
        nonlocal shutdown_event
        if shutdown_event:
            sys.stdout.write("forcing exit\n")
            sys.stdout.flush()
            sys.exit(1)
        shutdown_event = True
        sys.stdout.write("shutting down ...\n")
        sys.stdout.flush()
        server.shutdown()

    signal.signal(signal.SIGTERM, sighandler)
    signal.signal(signal.SIGINT, sighandler)

    pid = os.getpid()
    sys.stdout.write(f"server listening on {host}:{port} (pid {pid})\n")
    sys.stdout.flush()
    server.serve_forever()


if __name__ == "__main__":
    main()
