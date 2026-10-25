# One VPC per environment (plan 12.2). Application, workers and the managed database talk only over it.
resource "digitalocean_vpc" "this" {
  name        = "${var.name_prefix}-vpc"
  region      = var.region
  ip_range    = var.ip_range
  description = "DungeonFlux private network for ${var.name_prefix}"
}
