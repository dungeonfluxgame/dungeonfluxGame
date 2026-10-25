# Bastion Droplet (coordinator decision a): the only host that accepts SSH from outside the VPC, and only
# from the admin_cidrs allowlist and only with keys (no password, no VPN account). Humans have one account
# each with a FIDO2 hardware key (admin_users); CI uses the forward-only dfci account (ruling b).
#
# REPLACE THIS HOST MONTHLY (manifest recurring_operations): it has no outbound 80/443 unless package_mirror_cidrs is set and installs
# nothing at first boot, so replacement is its only patching. The CI key is restricted by sshd permitopen to the LITERAL private
# addresses of the application and worker hosts, written once at first boot: when one of those hosts is replaced CI deploys fail
# until this bastion is replaced too, in the same change. Command, from the environment root:
#   terraform apply -replace='module.bastion[0].module.droplet.digitalocean_droplet.this[0]'
#
# Application and worker
# Droplets accept SSH only from this host (modules/firewall, bastion_tag). Deploy scripts reach hosts with
# ssh -J (ProxyJump). The bastion holds no secret, no Docker and no application data.
resource "digitalocean_firewall" "bastion" {
  name = "${var.name_prefix}-bastion"
  tags = [var.bastion_tag]

  # Empty admin_cidrs means NO inbound rule: reach the bastion through the DigitalOcean console only.
  dynamic "inbound_rule" {
    for_each = length(var.admin_cidrs) > 0 ? [var.ssh_port] : []
    content {
      protocol         = "tcp"
      port_range       = tostring(inbound_rule.value)
      source_addresses = var.admin_cidrs
    }
  }

  # Onward SSH inside the VPC, plus DNS and NTP. No general outbound 80/443 (ruling d): a stolen CI key must not make the
  # bastion a proxy to the internet. Package mirrors are reachable only if package_mirror_cidrs is set.
  outbound_rule {
    protocol              = "tcp"
    port_range            = tostring(var.ssh_port)
    destination_addresses = [var.vpc_cidr]
  }
  outbound_rule {
    protocol              = "tcp"
    port_range            = "53"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
  outbound_rule {
    protocol              = "udp"
    port_range            = "53"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
  outbound_rule {
    protocol              = "udp"
    port_range            = "123"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  dynamic "outbound_rule" {
    for_each = length(var.package_mirror_cidrs) > 0 ? ["80", "443"] : []
    content {
      protocol              = "tcp"
      port_range            = outbound_rule.value
      destination_addresses = var.package_mirror_cidrs
    }
  }
}

module "droplet" {
  source = "../droplet"

  name_prefix               = var.name_prefix
  role                      = "bastion"
  droplet_count             = 1
  size                      = var.size
  image                     = var.image
  region                    = var.region
  vpc_id                    = var.vpc_id
  tags                      = [var.bastion_tag]
  ssh_key_fingerprints      = var.ssh_key_fingerprints
  ci_deploy_authorized_keys = var.ci_deploy_authorized_keys
  admin_users               = var.admin_users
  psql_image                = var.psql_image
  backups_enabled           = false
  reserved_ip_enabled       = true
  docker_version_pin        = var.docker_version_pin
  containerd_version_pin    = "not-used-by-the-bastion"
  ssh_target_ips            = var.ssh_target_ips
}
