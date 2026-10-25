# Staging environment root (plan 12, 18 phase B). Nothing here has prevent_destroy: staging can be torn down (coordinator decision c). STATUS: UNVERIFIED - terraform was not installed on the
# authoring machine, so `terraform fmt` and `terraform validate` have not been run. Nothing was applied.
locals {
  environment     = "staging"
  prevent_destroy = false
  app_tag         = "${var.name_prefix}-app"
  worker_tag      = "${var.name_prefix}-worker"
  bastion_tag     = "${var.name_prefix}-bastion"
}

resource "digitalocean_tag" "app" {
  name = local.app_tag
}

resource "digitalocean_tag" "worker" {
  name = local.worker_tag
}

resource "digitalocean_tag" "bastion" {
  count = var.bastion_enabled ? 1 : 0
  name  = local.bastion_tag
}

module "network" {
  source      = "../../modules/network"
  name_prefix = var.name_prefix
  region      = var.region
  ip_range    = var.vpc_ip_range
}

module "firewall" {
  source              = "../../modules/firewall"
  name_prefix         = var.name_prefix
  vpc_cidr            = module.network.ip_range
  app_tag             = digitalocean_tag.app.name
  worker_tag          = digitalocean_tag.worker.name
  bastion_tag         = var.bastion_enabled ? digitalocean_tag.bastion[0].name : ""
  public_http_sources = var.public_http_sources
  load_balancer_ids   = var.load_balancer_ids
  runtime_port_range  = var.runtime_port_range
  media_gateway_port  = var.media_gateway_port
}

module "bastion" {
  count  = var.bastion_enabled ? 1 : 0
  source = "../../modules/bastion"

  name_prefix               = var.name_prefix
  region                    = var.region
  vpc_id                    = module.network.id
  vpc_cidr                  = module.network.ip_range
  bastion_tag               = digitalocean_tag.bastion[0].name
  admin_cidrs               = var.admin_cidrs
  size                      = var.bastion_size
  image                     = var.droplet_image
  ssh_key_fingerprints      = var.ssh_key_fingerprints
  ci_deploy_authorized_keys = var.ci_deploy_authorized_keys
  admin_users               = var.admin_users
  psql_image                = var.psql_image
  ssh_target_ips            = concat(module.app_droplets.private_ips, module.worker_droplets.private_ips)
  package_mirror_cidrs      = var.package_mirror_cidrs
}

module "app_droplets" {
  source                    = "../../modules/droplet"
  name_prefix               = var.name_prefix
  role                      = "app"
  droplet_count             = var.app_droplet_count
  size                      = var.app_droplet_size
  image                     = var.droplet_image
  region                    = var.region
  vpc_id                    = module.network.id
  tags                      = [digitalocean_tag.app.name]
  ssh_key_fingerprints      = var.ssh_key_fingerprints
  ci_deploy_authorized_keys = var.ci_deploy_authorized_keys
  admin_users               = var.admin_users
  psql_image                = var.psql_image
  backups_enabled           = var.droplet_backups_enabled
  docker_version_pin        = var.docker_version_pin
  containerd_version_pin    = var.containerd_version_pin

  cosign_certificate_identity      = var.cosign_certificate_identity
  cosign_oidc_issuer               = var.cosign_oidc_issuer
  cosign_version                   = var.cosign_version
  cosign_sha256                    = var.cosign_sha256
  release_repository_prefix        = var.release_repository_prefix
  vpc_cidr                         = var.vpc_ip_range
  release_registry_user            = var.release_registry_user
  cosign_keys_certificate_identity = var.cosign_keys_certificate_identity
  edge_environment                 = {
    DF_PUBLIC_HOST     = var.public_hostname
    DF_ACME_EMAIL      = var.acme_email
    DF_ALLOWED_CIDRS   = join(" ", var.edge_allowed_cidrs)
    DF_TRUSTED_PROXIES = "private_ranges"
  }
}

module "worker_droplets" {
  source                    = "../../modules/droplet"
  name_prefix               = var.name_prefix
  role                      = "worker"
  droplet_count             = var.worker_droplet_count
  size                      = var.worker_droplet_size
  image                     = var.droplet_image
  region                    = var.region
  vpc_id                    = module.network.id
  tags                      = [digitalocean_tag.worker.name]
  ssh_key_fingerprints      = var.ssh_key_fingerprints
  ci_deploy_authorized_keys = var.ci_deploy_authorized_keys
  admin_users               = var.admin_users
  psql_image                = var.psql_image
  backups_enabled           = var.droplet_backups_enabled
  reserved_ip_enabled       = false
  docker_version_pin        = var.docker_version_pin
  containerd_version_pin    = var.containerd_version_pin

  cosign_certificate_identity      = var.cosign_certificate_identity
  cosign_oidc_issuer               = var.cosign_oidc_issuer
  cosign_version                   = var.cosign_version
  cosign_sha256                    = var.cosign_sha256
  release_repository_prefix        = var.release_repository_prefix
  vpc_cidr                         = var.vpc_ip_range
  release_registry_user            = var.release_registry_user
  cosign_keys_certificate_identity = var.cosign_keys_certificate_identity
}

module "database" {
  source              = "../../modules/database"
  name_prefix         = var.name_prefix
  region              = var.region
  vpc_id              = module.network.id
  engine_version      = var.database_engine_version
  size                = var.database_size
  node_count          = var.database_node_count
  storage_size_mib    = var.database_storage_size_mib
  database_name       = var.database_name
  trusted_droplet_ids = concat(module.app_droplets.ids, module.worker_droplets.ids)
  trusted_ip_cidrs    = var.database_trusted_ip_cidrs
  prevent_destroy     = local.prevent_destroy
  tags                = [local.environment, "dungeonflux"]
}

module "spaces" {
  source                = "../../modules/spaces"
  region                = var.region
  bucket_prefix         = var.bucket_prefix
  buckets_csv           = "${path.root}/../../../../deploy/storage/buckets.csv"
  cors_allowed_origins  = var.cors_allowed_origins
  prevent_destroy       = local.prevent_destroy
  cdn_for_public_static = var.cdn_for_public_static
}
