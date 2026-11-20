# Infrastructure (Terraform)

Everything for MongoDB Playground: GCP (Cloud Run, load balancer, Cloud Armor, Secret Manager, Cloud Build, monitoring), Atlas (M20 cluster, Private Service Connect, one DB user per job) and Cloudflare (DNS, TLS, origin key, rate limits, cache).

```
Cloudflare (proxy, WAF, rate limit, cache; sets X-Origin-Key)
  -> global HTTPS LB + Cloud Armor (OWASP rules off API paths, origin lock)
    -> Cloud Run "playground-api" (serves the SPA and /api/v1)
      -> Direct VPC egress -> Private Service Connect -> Atlas M20 (no public path)
```

## Credentials (environment only, never files in the repo)

```bash
export MONGODB_ATLAS_PUBLIC_API_KEY=...   # Atlas project owner key
export MONGODB_ATLAS_PRIVATE_API_KEY=...
export CLOUDFLARE_API_TOKEN=...           # Zone:Edit, DNS:Edit, Zone Settings:Edit, Rulesets:Edit
gcloud auth application-default login
```

## First deploy

1. Create the state bucket once (private, versioned): `gcloud storage buckets create gs://<project>-tfstate --location us-central1 --uniform-bucket-level-access` and `gcloud storage buckets update gs://<project>-tfstate --versioning`.
2. `cp terraform.tfvars.example terraform.tfvars` and edit.
3. `terraform init -backend-config="bucket=<project>-tfstate"`, `terraform plan`, `terraform apply`.
4. **Apply a second time.** The Atlas private-endpoint connection string only appears after the PSC connection is accepted, so the first apply leaves the database secrets as `pending`.
5. Load the sample data once: `atlas clusters sampleData load playground --projectId <atlas project>`.
6. Connect Cloud Build to GitHub (Terraform cannot do the browser step, so this is a script):
   1. `terraform apply` already granted Cloud Build's service agent `roles/secretmanager.admin` (variable `enable_github_connection_setup`, default true). The connection needs it to store the GitHub token as a secret.
   2. Run `scripts/setup-cloudbuild-github.sh` (equivalent to `gcloud builds connections create github github --region=us-central1`, then linking the repository). It prints a URL: open it, sign in to GitHub, install the Cloud Build GitHub App and select the repository (private repos work). The script waits for the authorisation, links the repo and prints `github_repository_id`.
   3. Put that value in `terraform.tfvars` and `terraform apply` again: this creates the `playground-deployer` service account and the trigger. Push to `main`: `cloudbuild.yaml` builds, tests and deploys the real image (the service starts on a placeholder image).
   4. Optional: set `enable_github_connection_setup = false` and apply to drop the service agent's Secret Manager grant.
7. Re-verify Cloudflare's IP ranges (`modules/edge`), watch Cloud Armor's preview logs for real traffic matching the origin rules, then set `enforce_origin_restriction = true`.

## Permissions

| Who | Needs | For |
|---|---|---|
| You, running `terraform apply` | Project **Owner**, or: Editor + Project IAM Admin + Service Account Admin + Secret Manager Admin + Cloud Build Editor + Cloud Run Admin + Compute Admin | Creating everything, including IAM bindings and the Cloud Build trigger |
| You, running `setup-cloudbuild-github.sh` | `roles/cloudbuild.connectionAdmin` (included in Owner), plus Secret Manager enabled | Creating the connection and linking the repository |
| Cloud Build service agent `service-<project number>@gcp-sa-cloudbuild.iam.gserviceaccount.com` | `roles/secretmanager.admin` (granted by Terraform) | Storing the GitHub token while the connection is created |
| `playground-deployer` (created by Terraform) | Artifact Registry writer, Logs writer, Cloud Run admin, Service Account User | Running `cloudbuild.yaml` |
| Atlas API key | Project Owner during the first apply, then Project Read Only | Cluster, private endpoint and database users |
| Cloudflare API token | Zone: Read, DNS Edit, Zone Settings Edit, Transform Rules Edit, Zone WAF Edit, Cache Rules Edit; Account: Turnstile Edit | DNS, TLS, rules and the Turnstile widget |

## Things to verify on the first real deploy

- The built-in roles for the `runner` user grant `dropDatabase`, `createIndex` and the search-index actions (`modules/atlas`); swap in a `mongodbatlas_custom_db_role` if not.
- Atlas Search on the M20: `GET /api/v1/limits` should report `searchSupported: true`.
- The Google-managed certificate can take a while to provision behind a proxied Cloudflare record.
- Turnstile is not wired in yet (the UI does not send tokens); the abuse controls are Cloudflare rate limits and the app's own limits.
