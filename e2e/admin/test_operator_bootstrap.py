"""A12: operator_bootstrap; governing contract: planning/account-service-research.md 10.1; planning/service-operations.md Recovery and operator access."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["operator_bootstrap"])
