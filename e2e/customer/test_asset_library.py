"""C42: asset_library; governing contract: planning/master-launch-plan.md 9.12,13.4-13.6; planning/privacy-compliance-review.md 3,6."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["asset_library"])
