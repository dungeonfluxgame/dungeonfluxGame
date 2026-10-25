"""C23: hero_choice; governing contract: planning/campaigns-and-table-research.md 8.4; planning/master-launch-plan.md 9.6,13.6."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["hero_choice"])
