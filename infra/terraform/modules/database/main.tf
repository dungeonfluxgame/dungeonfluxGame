# Managed PostgreSQL (plan 12.1, 12.3, 12.4). PostgreSQL is the authority for money and accepted game state.
# Backups: the managed service provides daily backups and point-in-time recovery inside its documented
# window; independent encrypted logical exports and restore drills are deploy/scripts/backup-*.sh and
# restore-drill.sh (plan 13.10).
#
# NO APPLICATION ROLES HERE (critic round 1, item 12). Terraform creates the cluster and the database only.
# The service roles (df_migrator, df_control, df_runner, df_campaign_runtime, df_egress, df_billing, df_support)
# are created by an operator with deploy/db/create-roles.sh from operator-generated password files, so no
# application password ever enters Terraform state or an output. What state DOES hold: the cluster resource
# exports the administrator's (doadmin) password, uri and private_uri, so the state bucket is a secret store
# (see backend.tf and deploy/secrets/secrets.manifest.json): private, versioned, readable only by the protected
# apply job, and its key is rotated when anyone who could read it leaves.
#
# prevent_destroy cannot be conditional in Terraform, so the cluster is declared twice and exactly one copy
# exists: "protected" (production) refuses destruction, "unprotected" (staging, coordinator decision c) can be
# torn down.
locals {
  cluster_args = {
    name                 = "${var.name_prefix}-pg"
    engine               = "pg"
    version              = var.engine_version
    size                 = var.size
    region               = var.region
    node_count           = var.node_count
    private_network_uuid = var.vpc_id
    storage_size_mib     = var.storage_size_mib
    tags                 = var.tags
  }
  cluster_id = one(concat(
    digitalocean_database_cluster.protected[*].id,
    digitalocean_database_cluster.unprotected[*].id,
  ))
  cluster_private_host = one(concat(
    digitalocean_database_cluster.protected[*].private_host,
    digitalocean_database_cluster.unprotected[*].private_host,
  ))
  cluster_port = one(concat(
    digitalocean_database_cluster.protected[*].port,
    digitalocean_database_cluster.unprotected[*].port,
  ))
}

resource "digitalocean_database_cluster" "protected" {
  count                = var.prevent_destroy ? 1 : 0
  name                 = local.cluster_args.name
  engine               = local.cluster_args.engine
  version              = local.cluster_args.version
  size                 = local.cluster_args.size
  region               = local.cluster_args.region
  node_count           = local.cluster_args.node_count
  private_network_uuid = local.cluster_args.private_network_uuid
  storage_size_mib     = local.cluster_args.storage_size_mib
  tags                 = local.cluster_args.tags

  maintenance_window {
    day  = var.maintenance_day
    hour = var.maintenance_hour
  }

  lifecycle {
    prevent_destroy = true
  }
}

resource "digitalocean_database_cluster" "unprotected" {
  count                = var.prevent_destroy ? 0 : 1
  name                 = local.cluster_args.name
  engine               = local.cluster_args.engine
  version              = local.cluster_args.version
  size                 = local.cluster_args.size
  region               = local.cluster_args.region
  node_count           = local.cluster_args.node_count
  private_network_uuid = local.cluster_args.private_network_uuid
  storage_size_mib     = local.cluster_args.storage_size_mib
  tags                 = local.cluster_args.tags

  maintenance_window {
    day  = var.maintenance_day
    hour = var.maintenance_hour
  }
}

resource "digitalocean_database_db" "app" {
  cluster_id = local.cluster_id
  name       = var.database_name
}

resource "digitalocean_database_firewall" "this" {
  cluster_id = local.cluster_id

  dynamic "rule" {
    for_each = toset(var.trusted_droplet_ids)
    content {
      type  = "droplet"
      value = rule.value
    }
  }

  dynamic "rule" {
    for_each = toset(var.trusted_ip_cidrs)
    content {
      type  = "ip_addr"
      value = rule.value
    }
  }
}

# TLS verification (plan 12.2): clients verify the server against this CA.
data "digitalocean_database_ca" "this" {
  cluster_id = local.cluster_id
}
