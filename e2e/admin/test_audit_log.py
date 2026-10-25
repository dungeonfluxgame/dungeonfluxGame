"""A11: audit_log; governing contract: planning/account-service-research.md 10,13.1; planning/observability.md."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["audit_log"])
