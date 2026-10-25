"""C08: mfa; governing contract: planning/master-launch-plan.md 14.1; planning/account-service-research.md 10.1 (operator only)."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["mfa"])
