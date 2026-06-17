import socket
import time
import sys


def main():
    host = "localhost"
    port = 8080
    name = sys.argv[1] if len(sys.argv) > 1 else "client"

    print(f"started (pid {__import__('os').getpid()})", flush=True)
    counter = 0

    # Keep a single persistent connection to avoid reconnect races
    while True:
        try:
            sock = socket.create_connection((host, port), timeout=3)
            break
        except (ConnectionRefusedError, socket.timeout, OSError) as e:
            print(f"waiting for server: {e}", flush=True)
            time.sleep(1)

    try:
        while True:
            counter += 1
            msg = f"request #{counter} from {name}\n"
            sock.sendall(msg.encode())
            print(f"sent: request #{counter} from {name}", flush=True)
            time.sleep(5)
    finally:
        sock.close()


if __name__ == "__main__":
    main()


