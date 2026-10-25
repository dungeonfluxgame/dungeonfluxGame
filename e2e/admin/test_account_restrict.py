"""A04: account_restrict; governing contract: planning/account-service-research.md 10.1; planning/commerce-service.md Account and tenant lifecycle."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["account_restrict"])
