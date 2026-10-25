"""Loopback HTTP boundary driver, deliberately separate from browser cookie evidence."""

import json
import secrets
from http.client import HTTPConnection
from http.cookies import SimpleCookie
from itertools import count
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPHandler, HTTPRedirectHandler, ProxyHandler, Request, build_opener

CLIENT_NUMBERS = count(secrets.randbelow(1 << 23))


class SyntheticSource(HTTPHandler):
    def __init__(self, number):
        super().__init__()
        self.source = (f"127.{number >> 16 & 255}.{number >> 8 & 255}.{number & 255}", 0)

    def http_open(self, request):
        return self.do_open(lambda host, **options: HTTPConnection(
            host, source_address=self.source, **options), request)


def require_loopback(origin):
    parsed = urlsplit(origin)
    if (parsed.scheme not in ("http", "https")
            or parsed.hostname not in ("127.0.0.1", "localhost", "::1")
            or parsed.port == 63309 or parsed.username or parsed.password
            or parsed.query or parsed.fragment or parsed.path not in ("", "/")):
        raise ValueError("FLOW-SAFETY: only dedicated loopback origins, never port 63309")
    return origin.rstrip("/")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


class Reply:
    def __init__(self, status, headers, body):
        self.status = status
        self.headers = headers
        self.body = body
        try:
            self.data = json.loads(body)
        except (ValueError, UnicodeDecodeError):
            self.data = None


class Client:
    def __init__(self, origin):
        self.origin = require_loopback(origin)
        self.cookies = SimpleCookie()
        self.csrf = ""
        self.opener = build_opener(ProxyHandler({}), NoRedirect(), SyntheticSource(next(CLIENT_NUMBERS)))

    def call(self, method, path, data=None, headers=None):
        if not path.startswith("/") or path.startswith("//") or "#" in path:
            raise ValueError("FLOW-SAFETY: HTTP path must be local and fragment-free")
        request_headers = {"Accept": "application/json"}
        if self.cookies:
            request_headers["Cookie"] = "; ".join(
                f"{name}={item.value}" for name, item in self.cookies.items())
        if method not in ("GET", "HEAD"):
            request_headers.update({"Origin": self.origin, "Sec-Fetch-Site": "same-origin",
                                    "Content-Type": "application/json", "X-DF-CSRF": self.csrf})
        for key, value in (headers or {}).items():
            if value is None:
                request_headers.pop(key, None)
            else:
                request_headers[key] = value
        payload = data if isinstance(data, bytes) else None if data is None else json.dumps(data).encode("utf-8")
        request = Request(self.origin + path, payload, request_headers, method=method)
        try:
            response = self.opener.open(request, timeout=15)
        except HTTPError as response:
            return self._consume(response)
        return self._consume(response)

    def _consume(self, response):
        with response:
            for header in response.headers.get_all("Set-Cookie", []):
                incoming = SimpleCookie(header)
                for name, item in incoming.items():
                    if item["max-age"] == "0":
                        self.cookies.pop(name, None)
                    else:
                        self.cookies[name] = item
            body = response.read(2 * 1024 * 1024 + 1)
            if len(body) > 2 * 1024 * 1024:
                raise AssertionError("FLOW-BOUNDARY: response exceeds 2 MiB test boundary")
            return Reply(response.status, response.headers, body)

    def session(self):
        reply = self.call("GET", "/app/api/session")
        if reply.status != 200 or not isinstance(reply.data, dict) or "csrf" not in reply.data:
            raise AssertionError(f"FLOW-STACK: session API returned {reply.status}")
        self.csrf = reply.data["csrf"]
        return reply.data

    def operation(self):
        reply = self.call("GET", "/app/api/operation")
        if reply.status != 200 or not isinstance(reply.data, dict):
            raise AssertionError(f"FLOW-STACK: operation API returned {reply.status}")
        return reply.data["operation"]
