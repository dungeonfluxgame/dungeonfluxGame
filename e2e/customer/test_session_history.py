"""C41: session_history; governing contract: planning/master-launch-plan.md 9.4,9.11; planning/integration-api-and-embeds-research.md 2.4."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["session_history"])
