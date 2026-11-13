"""What the browser tests share: running `jampot` processes, joining a
share and talking to the joiner's local server."""

import base64
import hashlib
import http.client
import json
import os
import queue
import re
import socket
import subprocess
import tempfile
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
BINARY = TARGET_DIR / "debug/jampot"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
TIMEOUT = 20.0


class Process:
    """A jampot process whose output lines are collected as they arrive."""

    def __init__(
        self, arguments: list[str], env: dict[str, str], stdin: str | None = None
    ) -> None:
        self.process: subprocess.Popen[str] = subprocess.Popen(
            [str(BINARY), *arguments],
            cwd=ROOT,
            env=env,
            stdin=subprocess.DEVNULL if stdin is None else subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if stdin is not None:
            pipe = self.process.stdin
            assert pipe is not None
            _ = pipe.write(stdin)
            pipe.close()
        self.lines: queue.Queue[str] = queue.Queue()
        self.seen: list[str] = []
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        stdout = self.process.stdout
        assert stdout is not None
        for line in stdout:
            self.lines.put(line.rstrip("\n"))

    def wait_for(self, prefix: str) -> str:
        """The rest of the first output line starting with `prefix`."""
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.1)
            except queue.Empty:
                if self.process.poll() is not None and self.lines.empty():
                    break
                continue
            self.seen.append(line)
            if line.startswith(prefix):
                return line.removeprefix(prefix)
        pytest.fail(f"no line starting with {prefix!r}; output: {self.seen}")

    def drain(self) -> list[str]:
        """The lines that arrived since the last look, without waiting."""
        new: list[str] = []
        while True:
            try:
                new.append(ANSI.sub("", self.lines.get_nowait()))
            except queue.Empty:
                break
        self.seen.extend(new)
        return new

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        try:
            _ = self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            _ = self.process.wait(timeout=5)


@dataclass(frozen=True)
class Shared:
    token: str
    address: str
    process: Process


@dataclass(frozen=True)
class Joined:
    url: str
    host: str
    port: int


def runtime_dir(home: Path) -> Path:
    # Unix socket paths are short: keep the runtime directory out of the
    # (long) pytest directory.
    digest = hashlib.sha256(str(home).encode()).hexdigest()[:10]
    return Path(tempfile.gettempdir()) / f"jj-{digest}"


def environment(home: Path) -> dict[str, str]:
    return {
        **os.environ,
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_RUNTIME_DIR": str(runtime_dir(home)),
    }


def jampot(home: Path, *arguments: str) -> str:
    """Runs a short jampot command and returns its output."""
    result = subprocess.run(
        [str(BINARY), *arguments],
        env=environment(home),
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
        check=True,
    )
    return result.stdout


@contextmanager
def share(home: Path, *source: str) -> Iterator[Shared]:
    process = Process([*source, "--local"], environment(home))
    try:
        token = process.wait_for("jampot: token: ")
        address = process.wait_for("jampot: direct address: ")
        yield Shared(token, address, process)
    finally:
        process.stop()


@contextmanager
def join_process(
    home: Path,
    shared: Shared,
    token: str,
    name: str | None = "alice",
    port: int | None = None,
) -> Iterator[Process]:
    # The token goes on stdin, as `jampot join -` reads it: out of argv.
    arguments = ["join", "-", "--no-open"]
    if name is not None:
        arguments += ["--name", name]
    if port is not None:
        arguments += ["--port", str(port)]
    process = Process(
        [*arguments, "--share-addr", shared.address],
        environment(home),
        stdin=f"{token}\n",
    )
    try:
        yield process
    finally:
        process.stop()


@contextmanager
def join(
    home: Path, shared: Shared, name: str | None = "alice", port: int | None = None
) -> Iterator[Joined]:
    with join_process(home, shared, shared.token, name, port) as process:
        url = process.wait_for("jampot: open ")
        parts = urlsplit(url)
        assert parts.hostname is not None and parts.port is not None
        joined = Joined(url, parts.netloc, parts.port)
        wait_connected(joined)
        yield joined


def request(
    joined: Joined,
    path: str,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    body: bytes | None = None,
) -> tuple[int, bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", joined.port, timeout=TIMEOUT)
    try:
        connection.request(
            method, path, body=body, headers={"Host": joined.host, **(headers or {})}
        )
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def wait_connected(joined: Joined) -> None:
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        try:
            if cast(dict[str, object], hello(joined)["link"])["connected"]:
                return
        except OSError:
            pass
        time.sleep(0.05)
    pytest.fail("the joiner was not admitted")


class PageSocket:
    """The joiner's page socket (`api/socket`), spoken by hand: what a page
    on the joiner's origin is told, as an oracle for the tests."""

    def __init__(self, joined: Joined, origin: str | None = None) -> None:
        self.sock: socket.socket = socket.create_connection(
            ("127.0.0.1", joined.port), timeout=TIMEOUT
        )
        key = base64.b64encode(os.urandom(16)).decode()
        origin = f"http://{joined.host}" if origin is None else origin
        handshake = (
            f"GET /_jampot/api/socket HTTP/1.1\r\nHost: {joined.host}\r\n"
            + "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            + f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
            + f"Origin: {origin}\r\n\r\n"
        )
        self.sock.sendall(handshake.encode())
        self.buffer: bytes = b""
        while b"\r\n\r\n" not in self.buffer:
            chunk = self.sock.recv(4096)
            if not chunk:
                break
            self.buffer += chunk
        head, _, self.buffer = self.buffer.partition(b"\r\n\r\n")
        self.status: int = int(head.split(b" ")[1]) if head else 0

    def _read(self, size: int) -> bytes:
        while len(self.buffer) < size:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise OSError("the socket closed")
            self.buffer += chunk
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def next(self) -> dict[str, object]:
        """The next message (the joiner's frames are unmasked and whole)."""
        while True:
            first, second = self._read(2)
            size = second & 0x7F
            if size == 126:
                size = int.from_bytes(self._read(2))
            elif size == 127:
                size = int.from_bytes(self._read(8))
            payload = self._read(size)
            opcode = first & 0x0F
            if opcode == 0x8:
                raise OSError("the joiner closed the socket")
            if opcode == 0x1:
                return cast(dict[str, object], json.loads(payload))

    def send(self, message: dict[str, object]) -> None:
        payload = json.dumps(message).encode()
        mask = os.urandom(4)
        size = len(payload)
        if size < 126:
            head = bytes([0x81, 0x80 | size])
        elif size < 1 << 16:
            head = bytes([0x81, 0x80 | 126]) + size.to_bytes(2)
        else:
            head = bytes([0x81, 0x80 | 127]) + size.to_bytes(8)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(head + mask + masked)

    def request(self, message: dict[str, object]) -> dict[str, object]:
        """Sends a request (numbered 1) and returns its reply."""
        self.send({**message, "n": 1})
        while True:
            reply = self.next()
            if reply["type"] == "reply" and reply["to"] == 1:
                return reply

    def close(self) -> None:
        self.sock.close()


def hello(joined: Joined) -> dict[str, object]:
    """What a page connecting now is told first: the joiner's state."""
    page = PageSocket(joined)
    try:
        first = page.next()
        assert first["type"] == "hello", first
        return first
    finally:
        page.close()
