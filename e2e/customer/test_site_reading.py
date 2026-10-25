"""C01: site_reading; governing contract: planning/master-launch-plan.md 9.1; docs/README.md."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["site_reading"])
