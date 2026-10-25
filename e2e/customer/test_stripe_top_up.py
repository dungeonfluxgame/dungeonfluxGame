"""C11: stripe_top_up; governing contract: planning/master-launch-plan.md 5.2,6.6,9.8; planning/account-service-research.md 8.2-8.4."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["stripe_top_up"])
