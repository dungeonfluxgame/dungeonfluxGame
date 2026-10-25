"""C15: payment_pending; governing contract: planning/master-launch-plan.md 6.6,9.8."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["payment_pending"])
