"""C03: verify_email; governing contract: planning/account-service-research.md 7.6; TENANTISSUE/round-2.md."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["verify_email"])
