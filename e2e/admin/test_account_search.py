"""A02: account_search; governing contract: planning/account-service-research.md 10.1-10.2."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["account_search"])
