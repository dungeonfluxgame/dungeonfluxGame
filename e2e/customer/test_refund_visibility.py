"""C16: refund_visibility; governing contract: planning/master-launch-plan.md 6.7,7.2,J13; planning/account-service-research.md 10.3."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["refund_visibility"])
