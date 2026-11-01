-- Funded admission revalidates exact current source winners and tombstones in its money transaction.
-- These catalog facts grant neither object credentials nor any source mutation capability.
GRANT SELECT ON media.tombstone,media.asset_variant TO df_imagery_admission;
