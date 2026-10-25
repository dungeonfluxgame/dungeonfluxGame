"""C30: tv_play; governing contract: planning/master-launch-plan.md 9.6; planning/campaigns-and-table-research.md 8.5; planning/remote-play.md."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["tv_play"])
