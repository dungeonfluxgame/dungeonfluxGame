-- Account-owned personal namespaces (account-service-research.md 5.2).
-- Match the explicit role grants of 0006; account-table RLS remains a deployment gate.
CREATE TABLE accounts.tenant (
    tenant_id bytea PRIMARY KEY DEFAULT uuid_send(gen_random_uuid())
        CHECK (octet_length(tenant_id) = 16 AND tenant_id <> decode(repeat('00', 16), 'hex')),
    owner_account_id bytea NOT NULL UNIQUE REFERENCES accounts.account(account_id),
    standing text NOT NULL DEFAULT 'active' CHECK (standing IN ('active', 'restricted')),
    restriction_reason text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (tenant_id <> owner_account_id),
    CHECK ((standing = 'restricted') = (restriction_reason IS NOT NULL)),
    CHECK (restriction_reason IS NULL OR length(restriction_reason) BETWEEN 1 AND 500)
);

GRANT SELECT, INSERT, UPDATE ON accounts.tenant TO df_accounts;
GRANT SELECT ON accounts.tenant TO df_campaigns;

-- Backfill once under the account migration lock, preserving any existing binding.
-- Closed/closing accounts retain their namespace, but the owner lookup denies its use.
INSERT INTO accounts.tenant (owner_account_id)
    SELECT account_id FROM accounts.account WHERE email_verified_at_ms IS NOT NULL
    ON CONFLICT (owner_account_id) DO NOTHING;
