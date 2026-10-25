# Every size and price decision is a variable (launch rule: configuration, not constants). Defaults are the
# plan's pilot values with the section that justifies each; reconfirm vendor prices and account eligibility
# before applying (plan 12.3, 12.4, 20). Nothing here is applied by agents. Variables without a default are
# decisions that must be made on purpose.

variable "region" {
  description = "Region slug where Droplets, managed PostgreSQL and Spaces are jointly available. NYC3 is a candidate in plan 12.2, not a claim about this account."
  type        = string
  default     = "nyc3"
}

variable "name_prefix" {
  description = "Prefix for every resource name."
  type        = string
  default     = "dfx-stage"
}

variable "vpc_ip_range" {
  description = "VPC CIDR. Staging and production use different ranges."
  type        = string
  default     = "10.20.0.0/20"
}

variable "ssh_key_fingerprints" {
  description = "Fingerprints of public SSH keys already registered in the DigitalOcean team. Required; public data."
  type        = list(string)
}

variable "ci_deploy_authorized_keys" {
  description = "Public keys of the CI deploy key (public data). Production deploys run only from CI, through the bastion, with this key; its forced command is df-deploy-ssh (coordinator ruling b)."
  type        = list(string)
}

variable "admin_users" {
  description = "Per-person operator accounts: name to FIDO2 sk-ssh-ed25519@openssh.com public keys (hardware keys; break-glass). No shared human account (ruling b). Required for any human access beyond the DigitalOcean console."
  type        = map(list(string))
  default     = {}
}

variable "psql_image" {
  description = "Digest-pinned one-shot psql client image (same major version as the cluster) that df-deploy runs for role and grant administration; CI keeps it equal to PSQL_IMAGE in deploy/images/images.lock."
  type        = string

  validation {
    condition     = can(regex("@sha256:[0-9a-f]{64}$", var.psql_image)) && can(regex("[:-](pg)?(1[7-9]|[2-9][0-9])([.-]|@)", var.psql_image))
    error_message = "psql_image must be pinned by digest and be a PostgreSQL 17 or later client."
  }
}

variable "bastion_enabled" {
  description = "Create the bastion Droplet (coordinator decision a). With it, application and worker Droplets accept SSH only from the bastion; without it there is no SSH rule anywhere (console access only)."
  type        = bool
  default     = true
}

variable "bastion_size" {
  description = "Bastion Droplet size slug (it only forwards SSH). Reconfirm the price at purchase; this is an addition to the plan 12.3 pilot bill."
  type        = string
  default     = "s-1vcpu-512mb-10gb"
}

variable "admin_cidrs" {
  description = "The owner's explicit allowlist of source CIDRs for SSH to the bastion. Empty: no inbound rule (console access only). 0.0.0.0/0 is refused."
  type        = list(string)
  default     = []
}

variable "public_http_sources" {
  description = "Sources allowed on 80/443 when no load balancer is in front. In STAGING this stays open on purpose: the certificate authority must reach the host to validate staging.dungeonfluxdnd.com. Access is restricted by the EDGE instead (DF_ALLOWED_CIDRS in deploy/config/edge.env.staging.example answers 403 outside the allowlist), because staging runs a fake identity provider."
  type        = list(string)
  default     = ["0.0.0.0/0", "::/0"]
}

variable "load_balancer_ids" {
  description = "Load balancer ids that front the application. Empty in the pilot (plan 12.3); the 12.4 baseline uses one regional load balancer (about 12 USD per month, a module added in a later round)."
  type        = list(string)
  default     = []
}

variable "runtime_port_range" {
  description = "Worker port range the control plane dials for the game tunnel. TODO(O1): confirm the runtime port model."
  type        = string
  default     = "20000-20999"
}

variable "media_gateway_port" {
  description = "Control-plane media gateway port (internal mTLS, decision 23). TODO(O1,O2): confirm."
  type        = string
  default     = "8083"
}

variable "droplet_image" {
  description = "Droplet image slug."
  type        = string
  default     = "ubuntu-24-04-x64"
}

variable "app_droplet_size" {
  description = "Application Droplet size slug. Plan 12.3 pilot: s-2vcpu-4gb."
  type        = string
  default     = "s-2vcpu-4gb"
}

variable "app_droplet_count" {
  description = "Application Droplets. Pilot 1 (not HA); the 12.4 baseline is 2."
  type        = number
  default     = 1
}

variable "worker_droplet_size" {
  description = "Worker Droplet size slug, used only if worker_droplet_count is above 0. Plan 12.3: separate workers onto another s-2vcpu-4gb when media processing or game memory interferes with webhooks and billing."
  type        = string
  default     = "s-2vcpu-4gb"
}

variable "worker_droplet_count" {
  description = "Worker Droplets. 0 in the pilot: the application Droplet also hosts the bounded campaign runtimes."
  type        = number
  default     = 0
}

variable "droplet_backups_enabled" {
  description = "Droplet daily backups, off in staging (production default is on; 30 percent of the Droplet price, plan 12.3). VM backups only, and they never contain a secret (secrets live on tmpfs); see plan 13.10."
  type        = bool
  default     = false
}

variable "docker_version_pin" {
  description = "Exact docker-ce and docker-ce-cli version (apt syntax). Required: an unpinned engine is never installed; the packages are held. Qualify a version, then set it."
  type        = string
}

variable "containerd_version_pin" {
  description = "Exact containerd.io version (apt syntax). Required; held after install."
  type        = string
}

variable "package_mirror_cidrs" {
  description = "Source networks of the package mirrors the bastion may reach on 80/443. Empty: no outbound 80/443 on the bastion (ruling d); it is patched by replacing it."
  type        = list(string)
  default     = []
}

variable "database_engine_version" {
  description = "PostgreSQL major version (configuration, coordinator decision f). The owner verifies that the account's DigitalOcean plan offers it, and pgvector on it, before the first apply."
  type        = string
  default     = "18"

  validation {
    condition     = can(tonumber(var.database_engine_version)) && tonumber(var.database_engine_version) >= 17
    error_message = "database_engine_version must be a PostgreSQL major version of 17 or later."
  }
}

variable "database_size" {
  description = "Managed PostgreSQL node size slug. Plan 12.3 pilot: db-s-1vcpu-2gb (Standard 2 GiB). Plan 12.4 conservative production: an Advanced 8 GiB / 2 vCPU primary. Account eligibility for Standard plans is an owner decision."
  type        = string
  default     = "db-s-1vcpu-2gb"
}

variable "database_node_count" {
  description = "1 = single node (pilot, no uptime SLA); 2 = primary plus standby if the account's plan allows."
  type        = number
  default     = 1
}

variable "database_storage_size_mib" {
  description = "Extra database storage in MiB, or null for the plan default."
  type        = number
  default     = null
}

variable "database_name" {
  description = "Application database name."
  type        = string
  default     = "dungeonflux"
}

variable "database_trusted_ip_cidrs" {
  description = "Extra trusted sources for the database (for example the bastion's address during maintenance). Empty: only the Droplets."
  type        = list(string)
  default     = []
}

variable "bucket_prefix" {
  description = "Prefix for Spaces bucket names; they are global to the region, so choose it deliberately (coordinator decision g). No default."
  type        = string
}

variable "cors_allowed_origins" {
  description = "Browser origins allowed on buckets whose cors column is read or upload (only public-static serves browsers directly; private media goes through the media gateway). Empty: no CORS rule."
  type        = list(string)
  default     = []
}

variable "cdn_for_public_static" {
  description = "CDN for the public-static bucket only."
  type        = bool
  default     = false
}

variable "public_hostname" {
  description = "The one public origin of the staging environment (staging.dungeonfluxdnd.com, LAUNCH.md decision 32). Written to the edge environment file as DF_PUBLIC_HOST. The staging root manages no DNS: the A record for it is in deploy/dns/records.csv (entered by hand at GoDaddy) or in the production root's optional dns module."
  type        = string
  default     = "staging.dungeonfluxdnd.com"
}
