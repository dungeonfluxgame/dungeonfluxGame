"""Synthetic identities created through the real account HTTP API and fake mail outbox."""

import re
import secrets
import time
from pathlib import Path

from e2e.http_client import Client

PASSWORD = "lantern under the flowmap mill"


def mail_token(outbox, email, purpose):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        for path in sorted(Path(outbox).glob("*.eml"), reverse=True):
            text = path.read_text(encoding="utf-8")
            if email not in text:
                continue
            match = re.search(re.escape(purpose) + r"#(?:token=)?([0-9a-f]+)", text)
            if match:
                return match.group(1)
        time.sleep(0.05)
    raise AssertionError("FLOW-MAIL: no matching synthetic mail within the bounded worker deadline")


def new_signup(client, email=None):
    return {"email": email or f"flowmap-{secrets.token_hex(8)}@example.test",
            "password": PASSWORD, "is_adult": True, "accepts_terms": True,
            "terms_version": "terms-2026-10", "operation": client.operation()}


def create_member(origin, outbox):
    client = Client(origin)
    client.session()
    body = new_signup(client)
    reply = client.call("POST", "/app/api/sign-up", body)
    assert reply.status == 200, f"FLOW-AUTH: signup fixture returned {reply.status}"
    token = mail_token(outbox, body["email"], "verify-email")
    reply = client.call("POST", "/app/api/verify-email",
                        {"token": token, "password": PASSWORD, "confirmation": PASSWORD})
    assert reply.status == 200, f"TENANTISSUE: verification fixture returned {reply.status}"
    client.session()
    reply = client.call("POST", "/app/api/sign-in",
                        {"email": body["email"], "password": PASSWORD})
    assert reply.status == 200, f"FLOW-AUTH: signin fixture returned {reply.status}"
    session = client.session()
    assert session.get("account", {}).get("is_verified"), "TENANTISSUE: fixture not verified"
    return client, body["email"]
