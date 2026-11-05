"""Minimal single-file Django service with the SAME endpoints as the others.

Endpoints (logically identical to the FastAPI and Seiva apps):
  GET  /health      -> 200 {"status": "ok"}
  POST /v1/notes    -> validate non-empty `title`, assign a monotonic id,
                       echo back {"id": <n>, "title": <title>}

No database (DATABASES is left empty), no templates, no admin, no sessions:
the "repository" is a process-local atomic counter, matching the Seiva
scaffold's in-memory repository + counter id-generator. Served as a WSGI app
by gunicorn (`django_app.app:application`), the usual production posture for
Django. We exercise the framework's request/response + JSON path, not I/O.
"""

import itertools
import json
import threading

from django.conf import settings
from django.core.wsgi import get_wsgi_application
from django.http import HttpResponseNotAllowed, JsonResponse
from django.urls import path
from django.views.decorators.csrf import csrf_exempt

if not settings.configured:
    settings.configure(
        DEBUG=False,
        ALLOWED_HOSTS=["*"],
        SECRET_KEY="benchmark-only-not-a-secret",
        ROOT_URLCONF=__name__,
        DATABASES={},
        INSTALLED_APPS=[],
        MIDDLEWARE=[],  # no session/auth/csrf middleware: match the trivial apps
        LOGGING_CONFIG=None,
    )

_counter = itertools.count(1)
_lock = threading.Lock()


def health(request):
    if request.method != "GET":
        return HttpResponseNotAllowed(["GET"])
    return JsonResponse({"status": "ok"})


@csrf_exempt
def create_note(request):
    if request.method != "POST":
        return HttpResponseNotAllowed(["POST"])
    try:
        body = json.loads(request.body)
        title = body["title"]
    except (ValueError, KeyError, TypeError):
        return JsonResponse({"detail": "invalid body"}, status=422)
    if not isinstance(title, str) or not title.strip():
        return JsonResponse({"detail": "title must not be empty"}, status=422)
    with _lock:
        new_id = next(_counter)
    return JsonResponse({"id": new_id, "title": title}, status=201)


urlpatterns = [
    path("health", health),
    path("v1/notes", create_note),
]

application = get_wsgi_application()
