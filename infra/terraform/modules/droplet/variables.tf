variable "name_prefix" {
  description = "Prefix for Droplet names."
  type        = string
}

variable "role" {
  description = "app, worker or bastion. Selects the cloud-init role and the name."
  type        = string

  validation {
    condition     = contains(["app", "worker", "bastion"], var.role)
    error_message = "role must be app, worker or bastion."
  }
}

variable "droplet_count" {
  description = "Number of Droplets in this role. Pilot (plan 12.3): 1 app, 0 workers. Conservative baseline (12.4): 2 app."
  type        = number

  validation {
    condition     = var.droplet_count >= 0 && floor(var.droplet_count) == var.droplet_count
    error_message = "droplet_count must be a non-negative integer."
  }
}

variable "size" {
  description = "Droplet size slug. Plan 12.3 pilot choice: s-2vcpu-4gb (about 24 USD per month); s-4vcpu-8gb for a larger node. Reconfirm the price at purchase; qualify capacity with measured memory and CPU (12.5)."
  type        = string
}

variable "image" {
  description = "Droplet base image slug. Pinned to an LTS slug; CI and deploy scripts never rely on a tag moving."
  type        = string
}

variable "region" {
  description = "DigitalOcean region slug."
  type        = string
}

variable "vpc_id" {
  description = "VPC UUID the Droplets join."
  type        = string
}

variable "tags" {
  description = "Tag names applied to the Droplets; firewalls select hosts by tag."
  type        = list(string)
}

variable "ssh_key_fingerprints" {
  description = "Fingerprints of public SSH keys already registered with the DigitalOcean team (public data, not secrets). They log in as root only until cloud-init disables root login."
  type        = list(string)
}

variable "ci_deploy_authorized_keys" {
  description = "Public keys of the CI deploy key (public data). On app and worker hosts each is installed for the dfdeploy account with the forced command df-deploy-ssh (only 'secrets-install' and 'deploy <digest>'); on the bastion for the dfci account as a forward-only key. Until df-deploy verifies releases the key is root-equivalent in effect (coordinator ruling b)."
  type        = list(string)
}

variable "admin_users" {
  description = "One entry per human operator: account name to list of public keys. Individual accounts only (no shared human account) and FIDO2 hardware keys only: every key must be sk-ssh-ed25519@openssh.com. They exist on the bastion and on app/worker hosts, where they may run df-secrets-install and df-deploy (including 'psql') through sudo and nothing else."
  type        = map(list(string))
  default     = {}

  validation {
    condition     = alltrue([for name, keys in var.admin_users : can(regex("^[a-z][a-z0-9_-]{1,31}$", name)) && !contains(["root", "dfdeploy", "dfci", "admin"], name)])
    error_message = "admin_users names must be lowercase account names and must not be root, dfdeploy, dfci or admin."
  }

  validation {
    condition     = alltrue([for name, keys in var.admin_users : length(keys) > 0 && alltrue([for k in keys : startswith(k, "sk-ssh-ed25519@openssh.com ")])])
    error_message = "every admin key must be a FIDO2 sk-ssh-ed25519@openssh.com public key (one account per person, hardware key required)."
  }
}

variable "psql_image" {
  description = "Digest-pinned reference of the one-shot psql client image that df-deploy may run for database administration (for example postgres:18.x@sha256:...). Required: it is the only image allowed before releases are verified."
  type        = string

  validation {
    condition     = can(regex("@sha256:[0-9a-f]{64}$", var.psql_image))
    error_message = "psql_image must be pinned by @sha256:<digest>."
  }

  validation {
    condition     = can(regex("[:-](pg)?(1[7-9]|[2-9][0-9])([.-]|@)", var.psql_image))
    error_message = "psql_image must be a PostgreSQL 17 or later client (the tag must carry the major version, for example postgres:18.1 or pgvector:0.8.1-pg18)."
  }
}

variable "backups_enabled" {
  description = "Droplet daily backups (30 percent of the Droplet price, plan 12.3). These back up the VM; they are not a database or object recovery plan (13.10), and they never contain secrets because secrets live on tmpfs."
  type        = bool
}

variable "monitoring_enabled" {
  description = "Install the DigitalOcean monitoring agent."
  type        = bool
  default     = true
}

variable "reserved_ip_enabled" {
  description = "Attach a reserved IP so replacing a Droplet keeps the address. There is no DNS record here: pointing DNS at the address is a manual owner step."
  type        = bool
  default     = true
}

variable "docker_version_pin" {
  description = "Exact docker-ce AND docker-ce-cli package version (apt syntax, for example 5:27.3.1-1~ubuntu.24.04~noble; both packages share it). Required: an unpinned engine is never installed, and the packages are held after install. Ignored for the bastion, which has no Docker."
  type        = string

  validation {
    condition     = length(var.docker_version_pin) > 0
    error_message = "docker_version_pin must be set to an exact docker-ce version."
  }
}

variable "containerd_version_pin" {
  description = "Exact containerd.io package version (apt syntax). Required for the same reason; held after install. Ignored for the bastion."
  type        = string

  validation {
    condition     = length(var.containerd_version_pin) > 0
    error_message = "containerd_version_pin must be set to an exact containerd.io version."
  }
}

variable "ssh_target_ips" {
  description = "Bastion only: the private addresses of the application and worker Droplets. The CI account on the bastion may tunnel ONLY to port 22 of these addresses (sshd permitopen takes literal addresses). Empty for other roles."
  type        = list(string)
  default     = []
}

variable "cosign_certificate_identity" {
  description = "The EXACT certificate identity (the signing workflow, for example https://github.com/OWNER/REPO/.github/workflows/release.yml@refs/heads/main) that df-deploy accepts for releases and SSH key sets. Pinned on the host; never a pattern. Required for the application and worker roles; unused by the bastion."
  type        = string
  default     = ""

  validation {
    condition     = var.cosign_certificate_identity == "" || can(regex("^https://github\\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/\\.github/workflows/[A-Za-z0-9._-]+@refs/(heads/main|tags/[A-Za-z0-9._/-]+)$", var.cosign_certificate_identity))
    error_message = "cosign_certificate_identity must be one exact GitHub Actions workflow identity on refs/heads/main or a tag (no wildcards)."
  }
}

variable "cosign_oidc_issuer" {
  description = "The OIDC issuer of that identity. GitHub Actions' issuer by default."
  type        = string
  default     = "https://token.actions.githubusercontent.com"
}

variable "cosign_version" {
  description = "The cosign release to install on application and worker hosts (a tag such as v2.4.1). Required for those roles."
  type        = string
  default     = ""
}

variable "cosign_sha256" {
  description = "SHA-256 of that release's cosign-linux-amd64 binary; the installer refuses a different file. Required for application and worker roles."
  type        = string
  default     = ""

  validation {
    condition     = var.cosign_sha256 == "" || can(regex("^[0-9a-f]{64}$", var.cosign_sha256))
    error_message = "cosign_sha256 must be 64 lowercase hex characters."
  }
}

variable "release_repository_prefix" {
  description = "The registry repository family of the images, for example ghcr.io/OWNER/dungeonflux: releases are <prefix>-release, key sets <prefix>-keys, images <prefix>-control-plane and so on. Pinned on the host. Required for application and worker roles."
  type        = string
  default     = ""
}
