"""A01: operator_login; governing contract: LAUNCH D34; planning/service-operations.md Recovery and operator access; account-service-research 10.1."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["operator_login"])
