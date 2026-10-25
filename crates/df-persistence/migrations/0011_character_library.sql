-- Personal character templates and campaign character bindings (planning/asset-storage-research.md
-- 9; plan 13.6, 15.1). Schema library. A template revision is immutable; saves compare-and-set on
-- current_revision so two tabs cannot silently overwrite one another. Progression, inventory and
-- currency live only in campaign game state, never here.

CREATE SCHEMA IF NOT EXISTS library;

CREATE TABLE library.character_template (
    template_id bytea PRIMARY KEY CHECK (octet_length(template_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    current_revision bigint NOT NULL CHECK (current_revision >= 1),
    created_at_ms bigint NOT NULL,
    archived_at_ms bigint,
    deleted_at_ms bigint
);
CREATE INDEX character_template_account ON library.character_template (account_id);

CREATE TABLE library.template_revision (
    template_id bytea NOT NULL REFERENCES library.character_template (template_id),
    revision bigint NOT NULL CHECK (revision >= 1),
    identity_record bytea NOT NULL CHECK (octet_length(identity_record) BETWEEN 1 AND 24576),
    identity_digest bytea NOT NULL CHECK (octet_length(identity_digest) = 32),
    rules_version text NOT NULL CHECK (octet_length(rules_version) <= 64),
    content_version text NOT NULL CHECK (octet_length(content_version) <= 64),
    portrait_asset bytea REFERENCES media.asset (asset_id),
    provenance text NOT NULL CHECK (provenance IN ('created', 'duplicated', 'imported',
        'saved_from_campaign', 'claimed_library')),
    provenance_source bytea CHECK (octet_length(provenance_source) BETWEEN 16 AND 64),
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (template_id, revision)
);

CREATE TABLE library.campaign_character (
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    character_id bytea NOT NULL CHECK (octet_length(character_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    origin_template bytea NOT NULL,
    origin_revision bigint NOT NULL,
    rules_version text NOT NULL CHECK (octet_length(rules_version) <= 64),
    created_at_ms bigint NOT NULL,
    left_at_ms bigint,
    PRIMARY KEY (campaign_id, character_id),
    FOREIGN KEY (origin_template, origin_revision)
        REFERENCES library.template_revision (template_id, revision)
);

CREATE FUNCTION library.refuse_revision_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'template revisions are immutable';
END $$;

CREATE TRIGGER template_revision_immutable BEFORE UPDATE OR DELETE
    ON library.template_revision FOR EACH ROW EXECUTE FUNCTION library.refuse_revision_change();
