"""C31: reconnect; governing contract: planning/runtime-reliability.md; planning/rpc-transport.md; master-launch-plan J08,J10."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["reconnect"])
