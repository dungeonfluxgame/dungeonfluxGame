"""A13: tunnel_access; governing contract: LAUNCH D34; planning/rpc-transport.md; planning/service-operations.md Release, access, restore steps and DNS."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["tunnel_access"])
