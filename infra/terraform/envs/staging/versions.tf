terraform {
  required_version = ">= 1.9.0, < 2.0.0"

  required_providers {
    digitalocean = {
      source = "digitalocean/digitalocean"
      # EXACT pin (critic round 1, item 19). The first `terraform init` by the owner or the protected apply
      # job must confirm this version exists (or bump it), and then .terraform.lock.hcl is generated and
      # committed with the provider checksums; the lock file could not be created on the authoring machine
      # (no terraform) and CI must fail while it is missing. The modules accept a range; the root decides.
      version = "= 2.40.0"
    }
  }
}

# No credentials in code. Supply them in the environment of the protected CI job or the operator shell:
#   DIGITALOCEAN_TOKEN                               API token (scoped to this environment)
#   SPACES_ACCESS_KEY_ID, SPACES_SECRET_ACCESS_KEY   Spaces key used to manage buckets
# The same Spaces key pair is NOT reused for runtime object access.
provider "digitalocean" {}
