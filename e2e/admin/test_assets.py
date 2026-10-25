"""A10: assets; governing contract: planning/master-launch-plan.md 9.13,13.4-13.5; planning/account-service-research.md 10.1."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["assets"])
