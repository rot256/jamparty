"""What a share can make the overlay keep is bounded: answers and their
dismissals as many as the joiner keeps, and cursors only of people here,
at most one each, removed once faded."""

import json
import uuid
from pathlib import Path

from playwright.sync_api import Page, expect
from support import SITE, SocketTap, join, overlay, share, waiting

MAX_ANSWERS = 100
MAX_JOINERS = 64


def answer(index: int) -> str:
    return json.dumps(
        {
            "type": "answer",
            "choices": [],
            "id": str(uuid.UUID(int=index + 1)),
            "html": f"<p>answer {index}</p>",
            "toYou": False,
        }
    )


def person(index: int, url: str | None) -> dict[str, object]:
    return {
        "name": f"p{index}",
        "fingerprint": f"{index:010x}",
        "key": f"{index + 1:064x}",
        "colour": index % 8,
        "url": url,
        "cursor": "on",
    }


def cursor(index: int) -> str:
    position = {"selector": "h1", "x": 5000, "y": 5000}
    return json.dumps(
        {"type": "cursor", "key": f"{index + 1:064x}", "url": "/", "position": position}
    )


def test_answers_and_dismissals_are_bounded(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        [route] = tap.sockets
        for index in range(MAX_ANSWERS + 50):
            route.send(answer(index))
        messages = overlay(page, ".agent-message")
        expect(messages.last).to_contain_text(f"answer {MAX_ANSWERS + 49}")
        expect(messages).to_have_count(MAX_ANSWERS)
        expect(messages.filter(has_text="answer 49")).to_have_count(0)
        expect(messages.filter(has_text="answer 50")).to_have_count(1)

        # Dismissals of answers the joiner no longer has are forgotten.
        stale = [str(uuid.UUID(int=index)) for index in range(1, 1000)]
        _ = page.evaluate(
            "(ids) => sessionStorage.setItem('jamparty-dismissed-v1', JSON.stringify(ids))",
            stale,
        )
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")

        def forgotten() -> bool:
            stored = page.evaluate("sessionStorage.getItem('jamparty-dismissed-v1')")
            return stored == "[]"

        waiting(page, "the dismissals to be forgotten", forgotten)


def test_cursors_only_of_people_here_and_bounded(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        [route] = tap.sockets
        cursors = overlay(page, ".cursor")

        # Keys that are not here draw nothing.
        for index in range(200):
            route.send(cursor(index))
        page.wait_for_timeout(300)
        expect(cursors).to_have_count(0)

        # More people here than a share takes: one cursor each, at most
        # as many as it takes.
        here = [person(index, "/") for index in range(MAX_JOINERS + 36)]
        route.send(json.dumps({"type": "people", "here": here}))
        for index in range(MAX_JOINERS + 36):
            route.send(cursor(index))
        expect(cursors.first).to_be_attached()
        page.wait_for_timeout(300)
        expect(cursors).to_have_count(MAX_JOINERS)

        # Faded out, they are removed.
        expect(cursors).to_have_count(0, timeout=10_000)
