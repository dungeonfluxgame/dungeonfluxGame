variable "name_prefix" {
  description = "Prefix for resource names."
  type        = string
}

variable "region" {
  description = "DigitalOcean region slug."
  type        = string
}

variable "vpc_id" {
  description = "VPC UUID."
  type        = string
}

variable "vpc_cidr" {
  description = "VPC CIDR; the bastion may open SSH connections only inside it."
  type        = string
}

variable "bastion_tag" {
  description = "Tag carried by the bastion; the application and worker firewalls accept SSH from this tag."
  type        = string
}

variable "admin_cidrs" {
  description = "Source CIDRs allowed to reach the bastion over SSH (the owner's allowlist). Empty: no inbound rule, console access only. Never 0.0.0.0/0 for production."
  type        = list(string)
  default     = []

  validation {
    condition     = !contains(var.admin_cidrs, "0.0.0.0/0") && !contains(var.admin_cidrs, "::/0")
    error_message = "admin_cidrs must be an explicit allowlist, not the whole internet."
  }
}

variable "ssh_port" {
  description = "SSH port."
  type        = number
  default     = 22
}

variable "size" {
  description = "Bastion Droplet size slug. A small shared-CPU size is enough (it only forwards SSH); reconfirm the price at purchase (plan 12.3 has no bastion line, so it is an addition to the pilot bill)."
  type        = string
}

variable "image" {
  description = "Droplet image slug."
  type        = string
}

variable "ssh_key_fingerprints" {
  description = "Fingerprints of public SSH keys registered in the DigitalOcean team (public data)."
  type        = list(string)
}

variable "ci_deploy_authorized_keys" {
  description = "Public keys of the CI deploy key (public data); on the bastion they belong to the forward-only dfci account."
  type        = list(string)
}

variable "admin_users" {
  description = "One entry per human operator: account name to FIDO2 sk-ssh-ed25519@openssh.com public keys. Individual accounts only; no shared human account."
  type        = map(list(string))
  default     = {}
}

variable "psql_image" {
  description = "Digest-pinned psql client image reference (written to allowed-images; unused on the bastion)."
  type        = string
}

variable "ssh_target_ips" {
  description = "Private addresses of the application and worker Droplets. The CI account may tunnel only to port 22 of these (permitopen), so a stolen CI key cannot use the bastion as a general proxy (ruling d). After a host is replaced its address changes: the bastion authorized_keys file must be refreshed (the round 2 key tool) or the bastion replaced."
  type        = list(string)
}

variable "package_mirror_cidrs" {
  description = "Source networks of the package mirrors the bastion may reach on 80/443. Empty (default) means NO outbound 80 or 443 at all (ruling d): the bastion installs nothing at first boot and is patched by replacing it. Set it only if you decide the bastion must update in place."
  type        = list(string)
  default     = []
}

variable "docker_version_pin" {
  description = "Unused by the bastion (no Docker); kept so the shared droplet module required inputs are satisfied."
  type        = string
  default     = "not-used-by-the-bastion"
}
