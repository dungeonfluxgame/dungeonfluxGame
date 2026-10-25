"""A06: refunds_adjustments; governing contract: planning/account-service-research.md 10.3; planning/master-launch-plan.md 6.7,7.3,J13."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["refunds_adjustments"])
