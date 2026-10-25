"""A07: sessions; governing contract: planning/master-launch-plan.md 9.13,16.4; planning/service-operations.md Recovery and operator access."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["sessions"])
