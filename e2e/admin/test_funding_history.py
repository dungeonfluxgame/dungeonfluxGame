"""A05: funding_history; governing contract: planning/account-service-research.md 10.2; planning/master-launch-plan.md 7.3,9.13."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["funding_history"])
