"""C12: paypal_top_up; governing contract: PAYPALTOPUP/round-1.md; planning/master-launch-plan.md 5.2,6.6,9.8."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["paypal_top_up"])
