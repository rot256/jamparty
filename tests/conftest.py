"""Fixtures for every browser test: the binary, the browsers and pages, and
the runtime directory each test leaves behind."""

import shutil
import subprocess
from collections.abc import Iterator
from pathlib import Path
from typing import cast

import pytest
from playwright.sync_api import Browser, Page, sync_playwright
from support import ROOT, runtime_dir


@pytest.fixture(autouse=True)
def remove_runtime_dir(tmp_path: Path) -> Iterator[None]:
    """Removes the test's runtime directory, which lives outside tmp_path."""
    yield
    shutil.rmtree(runtime_dir(tmp_path), ignore_errors=True)


@pytest.fixture(scope="session", autouse=True)
def build_binary() -> None:
    _ = subprocess.run(["cargo", "build"], cwd=ROOT, check=True)


@pytest.fixture(scope="session", params=["chromium", "firefox"])
def browser(request: pytest.FixtureRequest) -> Iterator[Browser]:
    with sync_playwright() as playwright:
        name = cast(str, request.param)
        browser = getattr(playwright, name).launch()
        assert isinstance(browser, Browser)
        yield browser
        browser.close()


@pytest.fixture()
def page(browser: Browser) -> Iterator[Page]:
    page = browser.new_page(viewport={"width": 1280, "height": 720})
    yield page
    page.close()
