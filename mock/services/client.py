import socket
import time
import sys


def main():
    host = "localhost"
    port = 8080
    name = sys.argv[1] if len(sys.argv) > 1 else "client"

    print(f"[{name}] started (pid {__import__('os').getpid()})", flush=True)
    counter = 0
    while True:
        counter += 1
        try:
            with socket.create_connection((host, port), timeout=3) as sock:
                msg = f"request #{counter} from {name}\n"
                sock.sendall(msg.encode())
                print(f"[{name}] sent: {msg.strip()}", flush=True)
        except (ConnectionRefusedError, socket.timeout, OSError) as e:
            print(f"[{name}] connection failed: {e}", flush=True)

        time.sleep(5)


if __name__ == "__main__":
    main()


