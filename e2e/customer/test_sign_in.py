"""C04: sign_in; governing contract: planning/account-service-research.md 7.2,7.6; LAUNCH D34."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["sign_in"])
