"""A08: support_queue; governing contract: planning/support-inbox-research.md 7-9,13 SUP-09,SUP-18,SUP-19."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["support_queue"])
