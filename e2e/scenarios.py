"""Executable cases distinguish boundary evidence from full-journey and manual evidence."""

from dataclasses import dataclass

from e2e.catalog import DIMENSIONS


@dataclass(frozen=True)
class Scenario:
    flow: str
    dimension: str
    variant: str


def scenarios_for(flow):
    cases = []
    for dimension in DIMENSIONS:
        if not applies(flow, dimension):
            continue
        variants = ("contract",)
        if dimension == "happy":
            variants = ("journey", "boundary") if flow.endpoint else ("journey",)
        elif dimension == "viewport":
            variants = ("375", "1280")
        elif dimension == "languages":
            variants = ("en", "es", "de", "fr")
        elif dimension == "security":
            variants = ("headers", "csrf", "origin", "fetch-site", "foreign-id")
        elif dimension == "permissions":
            variants = ("anonymous", "member", "host", "non-member", "stale-mfa")
        elif dimension == "accessibility":
            variants = ("names-keyboard", "screen-reader")
        elif dimension == "network":
            variants = ("offline", "lost-ack", "asset-failure")
        elif dimension == "concurrency":
            variants = ("two-tabs", "double-submit")
        elif dimension == "states":
            variants = ("empty", "loading", "unavailable")
        elif dimension == "time":
            variants = ("expiry", "idle-timeout", "reauth")
        elif dimension == "provider":
            variants = ("success", "cancel", "failure", "pending", "unknown", "refund")
        cases.extend(Scenario(flow.id, dimension, variant) for variant in variants)
    return cases


def applies(flow, dimension):
    public_read = flow.id in {"C01", "C18", "C36"}
    if public_read and dimension in {"permissions", "concurrency", "provider", "time", "idempotency", "audit"}:
        return False
    if dimension == "provider":
        return flow.fixture in {"payment", "oauth", "runtime"} or flow.id in {"C05", "C09", "C42", "A06", "A08", "A09"}
    return True
