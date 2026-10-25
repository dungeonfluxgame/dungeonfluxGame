# Cloud firewalls (plan 12.2; LAUNCH.md decisions 18 and 23). Only the reverse proxy receives public traffic;
# the database, worker control interface, debug tooling and admin services are never publicly reachable.
#
# What a host firewall CAN and CANNOT enforce: it sees hosts, not containers. "A runtime reaches only
# PostgreSQL, the dispatcher, the media gateway and the telemetry ingestor" is enforced by the Docker
# networks on each host (deploy/compose, internal networks with no default route; provider egress only from
# the dispatcher, which is a per-host process). This file adds the host-level part:
#   * workers get an outbound list of their own with NO runtime-port rule, so a worker-hosted runtime cannot
#     reach another worker's runtimes across the VPC; they reach the database and the app tier's media gateway;
#   * only the application tier may open connections to the runtime ports on workers;
#   * SSH only from the bastion.
locals {
  # Managed PostgreSQL listens on 25060 (direct) and 25061 (connection pool) inside the VPC.
  database_ports = "25060-25061"

  public_sources = length(var.load_balancer_ids) == 0 ? var.public_http_sources : []
  ssh_enabled    = var.bastion_tag != ""

  # What every host needs: DNS, package and provider HTTPS, NTP.
  outbound_base = [
    { protocol = "tcp", port_range = "53", addresses = ["0.0.0.0/0", "::/0"] },
    { protocol = "udp", port_range = "53", addresses = ["0.0.0.0/0", "::/0"] },
    { protocol = "tcp", port_range = "80", addresses = ["0.0.0.0/0", "::/0"] },
    { protocol = "tcp", port_range = "443", addresses = ["0.0.0.0/0", "::/0"] },
    { protocol = "udp", port_range = "123", addresses = ["0.0.0.0/0", "::/0"] },
    { protocol = "tcp", port_range = local.database_ports, addresses = [var.vpc_cidr] },
  ]

  # The application tier additionally dials the runtime ports on workers (the game tunnel).
  outbound_app = concat(local.outbound_base, [
    { protocol = "tcp", port_range = var.runtime_port_range, addresses = [var.vpc_cidr] },
  ])

  # Workers: the base list only. No runtime-port rule (runtime to runtime), no other VPC port.
  outbound_worker = local.outbound_base
}

resource "digitalocean_firewall" "app" {
  name = "${var.name_prefix}-app"
  tags = [var.app_tag]

  dynamic "inbound_rule" {
    for_each = length(local.public_sources) > 0 ? ["80", "443"] : []
    content {
      protocol         = "tcp"
      port_range       = inbound_rule.value
      source_addresses = local.public_sources
    }
  }

  dynamic "inbound_rule" {
    for_each = length(var.load_balancer_ids) > 0 ? ["80", "443"] : []
    content {
      protocol                  = "tcp"
      port_range                = inbound_rule.value
      source_load_balancer_uids = var.load_balancer_ids
    }
  }

  # The media gateway, for runtimes hosted on workers (internal mTLS). Not reachable from anywhere else.
  inbound_rule {
    protocol    = "tcp"
    port_range  = var.media_gateway_port
    source_tags = [var.worker_tag]
  }

  dynamic "inbound_rule" {
    for_each = local.ssh_enabled ? [var.ssh_port] : []
    content {
      protocol    = "tcp"
      port_range  = tostring(inbound_rule.value)
      source_tags = [var.bastion_tag]
    }
  }

  dynamic "outbound_rule" {
    for_each = local.outbound_app
    content {
      protocol              = outbound_rule.value.protocol
      port_range            = outbound_rule.value.port_range
      destination_addresses = outbound_rule.value.addresses
    }
  }

  outbound_rule {
    protocol              = "icmp"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}

resource "digitalocean_firewall" "worker" {
  name = "${var.name_prefix}-worker"
  tags = [var.worker_tag]

  # No public inbound at all: workers accept campaign runtime traffic only from the application tier.
  inbound_rule {
    protocol    = "tcp"
    port_range  = var.runtime_port_range
    source_tags = [var.app_tag]
  }

  dynamic "inbound_rule" {
    for_each = local.ssh_enabled ? [var.ssh_port] : []
    content {
      protocol    = "tcp"
      port_range  = tostring(inbound_rule.value)
      source_tags = [var.bastion_tag]
    }
  }

  dynamic "outbound_rule" {
    for_each = local.outbound_worker
    content {
      protocol              = outbound_rule.value.protocol
      port_range            = outbound_rule.value.port_range
      destination_addresses = outbound_rule.value.addresses
    }
  }

  # A worker-hosted runtime reaches the control plane's media gateway, and nothing else on the app tier.
  outbound_rule {
    protocol         = "tcp"
    port_range       = var.media_gateway_port
    destination_tags = [var.app_tag]
  }

  outbound_rule {
    protocol              = "icmp"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}
