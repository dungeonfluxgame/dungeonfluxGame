"""C02: sign_up; governing contract: planning/account-service-research.md 7.1,7.6; LAUNCH D28,D34."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["sign_up"])
