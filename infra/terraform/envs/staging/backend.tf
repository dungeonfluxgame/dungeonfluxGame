# Remote state in a private, versioned Spaces bucket that belongs to this environment only.
# THE STATE IS A SECRET STORE. The database cluster resource exports the administrator (doadmin) password, uri
# and private_uri into state, even though no output prints them. Rules: the bucket is readable only by the
# protected apply job (and the owner, individually, with MFA); no runtime or backup key can read it; it is
# versioned; and when anyone who could read it leaves, the administrator credential is reset. (Application
# role passwords are NOT in state: deploy/db/create-roles.sh creates the roles outside Terraform.)
# The bucket and key are NOT in this file. Initialise with a partial backend configuration:
#   terraform init -backend-config=backend.hcl        (backend.hcl is gitignored; see backend.hcl.example)
# and put credentials in AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY (the Spaces state key pair).
# The state bucket itself is created once by the owner, outside Terraform, to avoid a chicken-and-egg state.
terraform {
  backend "s3" {
    region                      = "us-east-1"
    skip_credentials_validation = true
    skip_requesting_account_id  = true
    skip_metadata_api_check     = true
    skip_region_validation      = true
    skip_s3_checksum            = true
    use_path_style              = false
  }
}
