"""C09: data_export; governing contract: planning/account-service-research.md 7.5-7.6; planning/privacy-compliance-review.md 5; planning/master-launch-plan.md J14,13.11."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["data_export"])
