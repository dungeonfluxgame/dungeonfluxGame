"""C06: google_sign_in; governing contract: planning/google-sign-in-research.md 3-5,7.4,Direct Google OIDC addendum."""

from e2e.catalog import BY_NAME
from e2e.scenarios import scenarios_for

CASES = scenarios_for(BY_NAME["google_sign_in"])
