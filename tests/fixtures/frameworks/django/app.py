"""A one-file Django site: a form that POSTs, with Django's CSRF checks."""

import sys

from django.conf import settings

settings.configure(
    DEBUG=True,
    SECRET_KEY="jamparty-test-only",
    ROOT_URLCONF=__name__,
    ALLOWED_HOSTS=["127.0.0.2"],
    MIDDLEWARE=["django.middleware.csrf.CsrfViewMiddleware"],
)

from django.core.management import execute_from_command_line
from django.http import HttpRequest, HttpResponse
from django.middleware.csrf import get_token
from django.urls import path
from django.utils.html import escape


def index(request: HttpRequest) -> HttpResponse:
    if request.method == "POST":
        said = escape(request.POST.get("said", ""))
        page = f'<!doctype html><title>Posted</title><h1 id="marker">posted {said}</h1>'
        return HttpResponse(page.encode())
    token = get_token(request)
    page = (
        '<!doctype html><title>Form</title><h1 id="marker">form</h1>'
        '<form method="post">'
        f'<input type="hidden" name="csrfmiddlewaretoken" value="{token}">'
        '<input name="said" value="hello"><button id="post">Post</button>'
        "</form>"
    )
    return HttpResponse(page.encode())


urlpatterns = [path("", index)]

if __name__ == "__main__":
    execute_from_command_line(sys.argv)
