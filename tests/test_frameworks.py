"""Real dev servers through `jampot proxy` and `jampot join`.

Each dev server listens on 127.0.0.2, where the page cannot reach it but
through jampot (as on another machine; on Linux all of 127/8 is loopback).
For each framework the page loads through the joiner with jampot's
overlay, editing a source file updates the page by itself, every
WebSocket the page opens goes to the page's own host and port, and no
console error is about origins, hosts or blocking.

Run with `uv run pytest -m frameworks` (Linux x86-64; Node.js 22.23 or
later and npm on PATH). npm projects are installed from their lockfiles
into a copy of the fixture; Hugo and Zola are downloaded once, checked
against their release's SHA-256, into $JAMPOT_TOOLS_DIR (default:
~/.cache/jampot-tests).
"""

import enum
import errno
import hashlib
import io
import os
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import time
import urllib.request
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import ConsoleMessage, Page, WebSocket, expect
from support import ROOT, jampot, join, join_process, share

pytestmark = pytest.mark.frameworks

FIXTURES = ROOT / "tests/fixtures/frameworks"
DEV_HOST = "127.0.0.2"
MARKER = re.compile(r"jampot-marker-\d+")
# Dev servers compile on first load; a loaded CI machine is slow.
LOAD = 90_000
# "By itself, within a few seconds", with room for a loaded machine.
UPDATE = 15_000
# Seconds with no request starting or ending after which a page with
# none in flight has settled.
QUIET = 1.0
STARTUP = 120.0
TOOLS = Path(os.environ.get("JAMPOT_TOOLS_DIR", Path.home() / ".cache/jampot-tests"))
COMPLAINT = re.compile(r"origin|host|blocked", re.IGNORECASE)


@dataclass(frozen=True)
class Download:
    """A release binary: where it is, its archive's SHA-256 (from the
    release's checksums), and the binary's path in the archive."""

    name: str
    url: str
    sha256: str
    member: str


HUGO = Download(
    "hugo-0.169.0",
    "https://github.com/gohugoio/hugo/releases/download/v0.169.0/hugo_0.169.0_linux-amd64.tar.gz",
    # hugo_0.169.0_checksums.txt
    "2d2f0bfe357d8cb4cec02f6bb28a3bff34978c8dda44b618029c5eb9d66765a7",
    "hugo",
)
ZOLA = Download(
    "zola-0.23.6",
    "https://github.com/getzola/zola/releases/download/v0.23.6/zola-v0.23.6-x86_64-unknown-linux-gnu.tar.gz",
    # The release asset's digest (Zola publishes no checksums file).
    "8f5132b3522412d04e395e0b25f6d68613ad272a873e54a2b3ebf664873024a4",
    "zola",
)


def tool(download: Download) -> Path:
    """The binary, downloaded and checked first if it is not here yet."""
    if (sys.platform, platform.machine()) != ("linux", "x86_64"):
        pytest.skip("the pinned Hugo and Zola binaries are for Linux x86-64")
    binary = TOOLS / download.name / download.member
    if binary.exists():
        return binary
    binary.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(download.url, timeout=120) as response:
        data = cast(bytes, response.read())
    digest = hashlib.sha256(data).hexdigest()
    assert digest == download.sha256, f"{download.url}: SHA-256 {digest}"
    # Only the binary is taken out, by name: no paths from the archive.
    with tarfile.open(fileobj=io.BytesIO(data)) as tar:
        member = tar.extractfile(download.member)
        assert member is not None, f"{download.member} not in {download.url}"
        partial = binary.with_suffix(".partial")
        _ = partial.write_bytes(member.read())
    partial.chmod(0o755)
    _ = partial.rename(binary)
    return binary


class Sockets(enum.Enum):
    """How the dev server's client hears of changes."""

    WEBSOCKET = enum.auto()
    # MkDocs long-polls over plain HTTP.
    POLLING = enum.auto()


class Address(enum.Enum):
    """Where the dev server listens."""

    DEV_HOST = enum.auto()
    # Eleventy's dev server takes no host: it listens on every address, so
    # the joiner cannot have its port and serves on another.
    EVERYWHERE = enum.auto()


Command = Callable[[Path, int], list[str]]


@dataclass(frozen=True)
class Framework:
    name: str
    # The command, given the site's directory and the port.
    command: Command
    # The file holding the marker.
    source: str
    sockets: Sockets = Sockets.WEBSOCKET
    # Requests that stay open until a change (long polls), not waited for.
    long_lived: tuple[str, ...] = ()
    address: Address = Address.DEV_HOST
    env: dict[str, str] = field(default_factory=dict)


def bin_(name: str) -> Command:
    """An npm package's command, run from the site's node_modules."""
    return lambda site, port: (
        [str(site / "node_modules/.bin" / name)]
        + [argument.format(host=DEV_HOST, port=port) for argument in COMMANDS[name]]
    )


COMMANDS = {
    "vite": ["--host", "{host}", "--port", "{port}", "--strictPort"],
    "next": ["dev", "--hostname", "{host}", "--port", "{port}"],
    # In the foreground, always: Astro runs a detached server when it
    # detects a coding agent, which `--ignore-lock` turns off.
    "astro": ["dev", "--host", "{host}", "--port", "{port}", "--ignore-lock"],
    "webpack": ["serve"],
    "eleventy": ["--serve", "--port", "{port}"],
}

FRAMEWORKS = [
    Framework(
        "hugo",
        lambda site, port: [
            str(tool(HUGO)),
            "server",
            "--source",
            str(site),
            "--bind",
            DEV_HOST,
            "--port",
            str(port),
        ],
        "layouts/home.html",
    ),
    Framework(
        "zola",
        lambda site, port: [
            str(tool(ZOLA)),
            *["--root", str(site), "serve", "--interface", DEV_HOST],
            *["--port", str(port)],
        ],
        "templates/index.html",
    ),
    Framework("vite", bin_("vite"), "label.js"),
    Framework(
        "next",
        bin_("next"),
        "app/page.js",
        env={"NEXT_TELEMETRY_DISABLED": "1"},
    ),
    Framework(
        "astro",
        bin_("astro"),
        "src/pages/index.astro",
        env={"ASTRO_TELEMETRY_DISABLED": "1"},
    ),
    Framework(
        "sveltekit",
        lambda site, port: [
            str(site / "node_modules/.bin/vite"),
            "dev",
            *["--host", DEV_HOST, "--port", str(port), "--strictPort"],
        ],
        "src/routes/+page.svelte",
    ),
    Framework("webpack", bin_("webpack"), "src/label.js"),
    Framework(
        "eleventy",
        bin_("eleventy"),
        "index.html",
        address=Address.EVERYWHERE,
    ),
    Framework(
        "mkdocs",
        lambda site, port: [
            sys.executable,
            "-m",
            "mkdocs",
            "serve",
            "--config-file",
            str(site / "mkdocs.yml"),
            "--dev-addr",
            f"{DEV_HOST}:{port}",
        ],
        "docs/index.md",
        sockets=Sockets.POLLING,
        long_lived=("/livereload/",),
    ),
]


def free_port() -> int:
    """A port free on 127.0.0.2, 127.0.0.1 and [::1] (as far as can be
    told): the dev server's, and the joiner's to mirror."""
    for _ in range(20):
        with socket.socket(socket.AF_INET) as probe:
            probe.bind((DEV_HOST, 0))
            port = cast(int, probe.getsockname()[1])
        try:
            for family, host in [
                (socket.AF_INET, "127.0.0.1"),
                (socket.AF_INET6, "::1"),
            ]:
                with socket.socket(family) as other:
                    other.bind((host, port))
        except OSError as error:
            if error.errno == errno.EADDRINUSE:
                continue
            # No IPv6 loopback: nobody has that side.
        return port
    pytest.fail("no port free on all loopback addresses")


@pytest.fixture(scope="session")
def sites(tmp_path_factory: pytest.TempPathFactory) -> Callable[[str], Path]:
    """A framework's fixture copied out and installed, once per session."""
    installed: dict[str, Path] = {}

    def site(name: str) -> Path:
        if name not in installed:
            target = tmp_path_factory.mktemp(name) / "site"
            _ = shutil.copytree(FIXTURES / name, target)
            if (target / "package-lock.json").exists():
                _ = subprocess.run(
                    ["npm", "ci", "--no-audit", "--no-fund", "--loglevel=error"],
                    cwd=target,
                    check=True,
                    timeout=600,
                )
            installed[name] = target
        return installed[name]

    return site


@contextmanager
def dev_server(
    command: list[str], site: Path, port: int, log: Path, env: dict[str, str]
) -> Iterator[str]:
    """Runs the dev server until it answers on DEV_HOST:port."""
    with log.open("w") as output:
        process = subprocess.Popen(
            command,
            cwd=site,
            env={**os.environ, "HOST": DEV_HOST, "PORT": str(port), **env},
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    try:
        deadline = time.monotonic() + STARTUP
        while True:
            if process.poll() is not None:
                pytest.fail(f"{command[0]} exited: {log.read_text()[-2000:]}")
            try:
                socket.create_connection((DEV_HOST, port), timeout=1).close()
                break
            except OSError:
                if time.monotonic() > deadline:
                    pytest.fail(f"{command[0]} never listened: {log.read_text()}")
                time.sleep(0.2)
        yield f"http://{DEV_HOST}:{port}"
    finally:
        # The whole group: some dev servers start children of their own.
        try:
            os.killpg(process.pid, signal.SIGTERM)
            _ = process.wait(timeout=10)
        except ProcessLookupError:
            pass
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            _ = process.wait(timeout=10)


@contextmanager
def edited(path: Path) -> Iterator[Callable[[str], None]]:
    """Replaces the marker in `path`; restores the file afterwards."""
    original = path.read_text()
    assert MARKER.search(original), f"no marker in {path}"

    def to(marker: str) -> None:
        _ = path.write_text(MARKER.sub(marker, original))

    try:
        yield to
    finally:
        _ = path.write_text(original)


@dataclass
class Watched:
    """What the page did: the WebSockets it opened, its console errors,
    the site's requests (and WebSockets) still in flight, and when one
    last started or ended."""

    sockets: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    in_flight: dict[object, str] = field(default_factory=dict)
    last: float = field(default_factory=time.monotonic)


def watch(page: Page, long_lived: tuple[str, ...] = ()) -> Watched:
    """Watches `page`; requests whose URL holds one of `long_lived` (a long
    poll that stays open until a change) are never waited for."""
    watched = Watched()

    def started(request: object, url: str) -> None:
        # jampot's own (events, presence, cursors) are not the site's.
        if "/_jampot/" in url or any(part in url for part in long_lived):
            return
        watched.in_flight[request] = url
        watched.last = time.monotonic()

    def ended(request: object) -> None:
        if watched.in_flight.pop(request, None) is not None:
            watched.last = time.monotonic()

    def on_socket(socket: WebSocket) -> None:
        watched.sockets.append(socket.url)
        # Settled once open: a frame either way says it is (Playwright
        # tells no open), as does a close.
        started(socket, socket.url)
        socket.on("framesent", lambda _: ended(socket))
        socket.on("framereceived", lambda _: ended(socket))
        socket.on("close", lambda _: ended(socket))
        socket.on("socketerror", lambda _: ended(socket))

    def on_console(message: ConsoleMessage) -> None:
        if message.type == "error":
            watched.errors.append(message.text)

    page.on("request", lambda request: started(request, request.url))
    page.on("requestfinished", ended)
    page.on("requestfailed", ended)
    page.on("websocket", on_socket)
    page.on("console", on_console)
    return watched


def settled(watched: Watched) -> bool:
    """Nothing of the site's in flight, nor started or ended for a while."""
    return not watched.in_flight and time.monotonic() - watched.last >= QUIET


def settle(page: Page, watched: Watched) -> None:
    """Waits until the page has settled: its scripts loaded and the dev
    server's client connected (hydration can take seconds on a loaded
    machine), so an edit is not made under a page still starting."""
    deadline = time.monotonic() + LOAD / 1000
    while not settled(watched):
        assert time.monotonic() < deadline, (
            f"the page never settled; in flight: {list(watched.in_flight.values())}"
        )
        page.wait_for_timeout(100)


def own_host(page: Page) -> str:
    return urlsplit(page.url).netloc


@pytest.mark.parametrize(
    "framework", FRAMEWORKS, ids=[framework.name for framework in FRAMEWORKS]
)
def test_live_reload_through_jampot(
    framework: Framework,
    sites: Callable[[str], Path],
    tmp_path: Path,
    page: Page,
) -> None:
    site = sites(framework.name)
    port = free_port()
    command = framework.command(site, port)
    log = tmp_path / "dev-server.log"
    with (
        edited(site / framework.source) as edit,
        dev_server(command, site, port, log, framework.env) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        match framework.address:
            case Address.DEV_HOST:
                # The joiner serves on the dev server's port.
                assert joined.port == port
            case Address.EVERYWHERE:
                assert joined.port != port
        watched = watch(page, framework.long_lived)
        _ = page.goto(joined.url, timeout=LOAD)
        expect(page.locator("#jampot")).to_be_attached(timeout=LOAD)
        marker = page.locator("#marker")
        expect(marker).to_have_text(MARKER, timeout=LOAD)
        # Let the dev server's client connect before the change.
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "green"
        )
        settle(page, watched)

        changed = f"jampot-marker-{time.monotonic_ns()}"
        edit(changed)
        expect(marker).to_have_text(changed, timeout=UPDATE)

        match framework.sockets:
            case Sockets.WEBSOCKET:
                assert watched.sockets, "the page opened no WebSocket"
            case Sockets.POLLING:
                pass
        own = own_host(page)
        for url in watched.sockets:
            assert urlsplit(url).netloc == own, f"{url} is not {own}"
        complaints = [error for error in watched.errors if COMPLAINT.search(error)]
        assert not complaints, complaints


def test_a_django_form_post_through_jampot(
    sites: Callable[[str], Path], tmp_path: Path, page: Page
) -> None:
    """Django's CSRF check compares Origin with the host it serves: the
    dev server must see its own origin."""
    site = sites("django")
    port = free_port()
    command = [
        sys.executable,
        str(site / "app.py"),
        "runserver",
        f"{DEV_HOST}:{port}",
        "--noreload",
    ]
    with (
        dev_server(command, site, port, tmp_path / "django.log", {}) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        assert joined.port == port
        watched = watch(page)
        _ = page.goto(joined.url, timeout=LOAD)
        expect(page.locator("#jampot")).to_be_attached()
        expect(page.locator("#marker")).to_have_text("form")
        page.locator("#post").click()
        # Django answered the POST: its CSRF check passed.
        expect(page.locator("#marker")).to_have_text("posted hello")
        complaints = [error for error in watched.errors if COMPLAINT.search(error)]
        assert not complaints, complaints


def test_a_dev_server_port_taken_here(
    sites: Callable[[str], Path], tmp_path: Path, page: Page
) -> None:
    """Hugo's reload client dials Hugo's port on the page's host: with that
    port taken on the joiner's machine, the joiner serves elsewhere, says
    so, and the page still loads (without live reload: the note says
    why)."""
    site = sites("hugo")
    port = free_port()
    command = [
        str(tool(HUGO)),
        "server",
        *["--source", str(site), "--bind", DEV_HOST, "--port", str(port)],
    ]
    with (
        socket.socket(socket.AF_INET) as taken,
        dev_server(command, site, port, tmp_path / "hugo.log", {}) as upstream,
    ):
        taken.bind(("127.0.0.1", port))
        taken.listen()
        with (
            share(tmp_path, "proxy", upstream) as shared,
            join_process(tmp_path, shared, shared.token) as process,
        ):
            note = process.wait_for("jampot: port ")
            assert note.startswith(f"{port} is in use here; serving on "), note
            url = process.wait_for("jampot: open ")
            assert urlsplit(url).port != port
            _ = page.goto(url, timeout=LOAD)
            expect(page.locator("#jampot")).to_be_attached()
            expect(page.locator("#marker")).to_have_text(MARKER)


def test_a_comment_draft_survives_a_dev_server_reload(
    sites: Callable[[str], Path], tmp_path: Path, page: Page
) -> None:
    """Hugo reloads the page itself, whatever jampot's dialog holds: the
    half-written comment comes back, and goes out once."""
    site = sites("hugo")
    port = free_port()
    command = [
        str(tool(HUGO)),
        "server",
        *["--source", str(site), "--bind", DEV_HOST, "--port", str(port)],
    ]
    with (
        edited(site / "layouts/home.html") as edit,
        dev_server(command, site, port, tmp_path / "hugo.log", {}) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        watched = watch(page)
        _ = page.goto(joined.url, timeout=LOAD)
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "green"
        )
        settle(page, watched)
        page.locator("#marker").click(button="right")
        textarea = page.locator("#jampot textarea")
        expect(textarea).to_be_focused()
        textarea.press_sequentially("Half written")
        changed = f"jampot-marker-{time.monotonic_ns()}"
        edit(changed)
        expect(page.locator("#marker")).to_have_text(changed, timeout=UPDATE)
        expect(textarea).to_have_value("Half written")
        expect(page.locator("#jampot #dialog-label")).to_have_text("Comment:")
        textarea.press_sequentially(", now whole.")
        textarea.press("Enter")
        expect(page.locator("#jampot #badge")).to_have_text("1 queued")
        lines = jampot(tmp_path, "recv").splitlines()
        assert len(lines) == 1, lines
        assert '"comment":"Half written, now whole."' in lines[0]
        assert '"selector":"#marker"' in lines[0]
