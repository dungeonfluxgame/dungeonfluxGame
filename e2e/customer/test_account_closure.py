"""C10: account_closure; governing contract: planning/account-service-research.md 7.5-7.6; planning/master-launch-plan.md J14."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["account_closure"])
