locals {
  allowed_users = join(" ", concat([var.role == "bastion" ? "dfci" : "dfdeploy"], keys(var.admin_users)))

  user_data = templatefile("${path.module}/../../../cloud-init/droplet.yaml.tftpl", {
    role                             = var.role
    ci_deploy_authorized_keys        = var.ci_deploy_authorized_keys
    admin_users                      = var.admin_users
    allowed_users                    = local.allowed_users
    docker_version_pin               = var.docker_version_pin
    containerd_version_pin           = var.containerd_version_pin
    ssh_target_ips                   = var.ssh_target_ips
    cosign_certificate_identity      = var.cosign_certificate_identity
    cosign_oidc_issuer               = var.cosign_oidc_issuer
    cosign_version                   = var.cosign_version
    cosign_sha256                    = var.cosign_sha256
    release_repository_prefix        = var.release_repository_prefix
    keys_apply_script                = file("${path.module}/../../../cloud-init/df-keys-apply.sh")
    slot_dirs_script                 = file("${path.module}/../../../cloud-init/df-slot-dirs.sh")
    net_rules_script                 = file("${path.module}/../../../cloud-init/df-net-rules.sh")
    vpc_cidr                         = var.vpc_cidr
    edge_environment                 = var.edge_environment
    release_registry_user            = var.release_registry_user
    cosign_keys_certificate_identity = var.cosign_keys_certificate_identity
    psql_image                       = var.psql_image
    secrets_install_script           = file("${path.module}/../../../cloud-init/df-secrets-install.sh")
    deploy_script                    = file("${path.module}/../../../cloud-init/df-deploy.sh")
    deploy_ssh_script                = file("${path.module}/../../../cloud-init/df-deploy-ssh.sh")
  })
}

resource "digitalocean_droplet" "this" {
  count = var.droplet_count

  name       = "${var.name_prefix}-${var.role}-${count.index + 1}"
  region     = var.region
  size       = var.size
  image      = var.image
  vpc_uuid   = var.vpc_id
  ssh_keys   = var.ssh_key_fingerprints
  tags       = var.tags
  backups    = var.backups_enabled
  monitoring = var.monitoring_enabled
  ipv6       = false
  user_data  = local.user_data

  lifecycle {
    precondition {
      condition = var.role == "bastion" || (
        var.cosign_certificate_identity != "" && var.cosign_keys_certificate_identity != "" && var.cosign_certificate_identity != var.cosign_keys_certificate_identity &&
        var.cosign_version != "" && var.cosign_sha256 != "" && var.release_repository_prefix != "" && var.vpc_cidr != ""
      )
      error_message = "Application and worker hosts need cosign_certificate_identity, a DIFFERENT cosign_keys_certificate_identity (key sets are signed by their own workflow), cosign_version, cosign_sha256, release_repository_prefix and vpc_cidr: df-deploy verifies every release and key set against them and the host firewall protects the VPC."
    }

    # Cloud-init runs once at creation. Later configuration changes go through the deploy scripts, never by
    # replacing a host that holds running campaigns. Image drift is handled by rebuilding deliberately.
    ignore_changes = [user_data, image]
  }
}

resource "digitalocean_reserved_ip" "this" {
  count  = var.reserved_ip_enabled ? var.droplet_count : 0
  region = var.region
}

resource "digitalocean_reserved_ip_assignment" "this" {
  count      = var.reserved_ip_enabled ? var.droplet_count : 0
  ip_address = digitalocean_reserved_ip.this[count.index].ip_address
  droplet_id = digitalocean_droplet.this[count.index].id
}
