#!/usr/bin/env python3
"""Forward one Podman Unix HTTP connection while injecting one deterministic inspect fault.

This fixture is intentionally small and local-only.  It never interprets Podman payloads and
forwards every request except the named container inspect endpoint.  The live harness uses it to
prove BoxFerry's handling of malformed and disappeared selected observations without racing a
real resource deletion.
"""

from __future__ import annotations

import argparse
import os
import socket
import socketserver
import sys

SOCKET_TIMEOUT_SECONDS = 30


def force_connection_close(request: bytes) -> bytes:
    head, separator, body = request.partition(b"\r\n\r\n")
    if not separator:
        return request
    lines = [
        line
        for line in head.split(b"\r\n")
        if not line.lower().startswith(b"connection:")
    ]
    lines.append(b"Connection: close")
    return b"\r\n".join(lines) + separator + body


def response(mode: str) -> bytes:
    if mode == "malformed":
        return b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{"
    if mode == "section-500":
        body = b'{"message":"simulated volume inventory failure"}'
        return (
            b"HTTP/1.1 500 Internal Server Error\r\n"
            b"Content-Type: application/json\r\n"
            + f"Content-Length: {len(body)}\r\n".encode("ascii")
            + b"Connection: close\r\n\r\n"
            + body
        )
    return b"HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"message\":\"gone\"}"


class Handler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        self.request.settimeout(SOCKET_TIMEOUT_SECONDS)
        request = bytearray()
        while b"\r\n\r\n" not in request:
            chunk = self.request.recv(65536)
            if not chunk:
                return
            request.extend(chunk)
        first_line = bytes(request).split(b"\r\n", 1)[0].decode("ascii", "replace")
        target = first_line.split(" ")[1] if len(first_line.split(" ")) >= 2 else ""
        needles = (
            f"/containers/{self.server.container}/json",  # type: ignore[attr-defined]
            f"/containers/{self.server.container_id}/json",  # type: ignore[attr-defined]
        )
        inject = any(needle in target for needle in needles)
        if self.server.mode == "section-500":  # type: ignore[attr-defined]
            inject = target.split("?", 1)[0].endswith("/libpod/volumes/json")
        if inject:
            self.request.sendall(response(self.server.mode))  # type: ignore[attr-defined]
            return
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as upstream:
            upstream.settimeout(SOCKET_TIMEOUT_SECONDS)
            upstream.connect(self.server.upstream)  # type: ignore[attr-defined]
            upstream.sendall(force_connection_close(bytes(request)))
            while True:
                chunk = upstream.recv(65536)
                if not chunk:
                    break
                self.request.sendall(chunk)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", required=True)
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--container", required=True)
    parser.add_argument("--container-id", required=True)
    parser.add_argument("--mode", choices=("malformed", "gone", "section-500"), required=True)
    args = parser.parse_args()
    if os.path.exists(args.listen):
        raise SystemExit(f"refusing to replace existing socket: {args.listen}")
    server = socketserver.ThreadingUnixStreamServer(args.listen, Handler)
    server.upstream = args.upstream
    server.container = args.container
    server.container_id = args.container_id
    server.mode = args.mode
    try:
        server.serve_forever()
    finally:
        server.server_close()
        if os.path.exists(args.listen):
            os.unlink(args.listen)
    return 0


if __name__ == "__main__":
    sys.exit(main())
