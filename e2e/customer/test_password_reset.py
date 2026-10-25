"""C05: password_reset; governing contract: planning/account-service-research.md 7.3,7.6."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["password_reset"])
