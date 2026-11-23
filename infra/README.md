# Infrastructure (Terraform)

Everything for MongoDB Playground: GCP (Cloud Run, load balancer, Cloud Armor, Secret Manager, Cloud Build, monitoring), Atlas (M10 cluster, Private Service Connect, one DB user per job) and Cloudflare (DNS, TLS, origin key, rate limits, cache).

```
Cloudflare (proxy, WAF, rate limit, cache; sets X-Origin-Key)
  -> global HTTPS LB + Cloud Armor (OWASP rules off API paths, origin lock)
    -> Cloud Run "playground-api" (serves the SPA and /api/v1)
      -> Direct VPC egress -> Private Service Connect -> Atlas M10 (no public path)
```

## No credentials in state, plans or the repo

- **Database passwords are never stored.** The four Atlas users (`pg-runner`, `pg-shares`, `pg-stats`, `pg-sample`) get passwords from `ephemeral "random_password"` resources, passed to Atlas (`password_wo`) and Secret Manager (`secret_data_wo`) as write-only values: they live for one run and are not written to the state or to a saved plan. So a plan file that is committed by mistake holds no database credential (and the Atlas "credentials found on GitHub" alert has nothing to find). Needs Terraform 1.11 or later.
- **Rotate** all four by increasing `credential_version` in `terraform.tfvars` and applying; the users and the connection-string secrets change in the same apply. Then restart the service (`gcloud run services update playground-api --region us-central1 --update-env-vars=REDEPLOY=$(date +%s)`).
- **Keep plans out of the repo:** use `scripts/tf.sh plan` and `scripts/tf.sh apply`. It writes the plan to a private directory outside the checkout, applies exactly that plan and deletes it. A plan file still holds the other generated secrets (origin key, tokens), so never commit one.
- **Guard rail:** `scripts/install-hooks.sh` enables a pre-commit hook that refuses plan/state/`.env`/tfvars files and anything that looks like a credential (connection string with a password, private key, API token).

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
   2. Run `scripts/setup-cloudbuild-github.sh` (equivalent to `gcloud builds connections create github github --region=us-central1`, then linking the repository). It prints a URL: open it, sign in to GitHub and install the Cloud Build GitHub App on the account that owns the repository (private repos work). Under **Repository access**, choose **Only select repositories** and add `mongodb-playground`, or choose **All repositories**; if the app cannot see the repo, linking fails with "repository ... does not exist or is not accessible to the Cloud Build GitHub App". Fix it at `https://github.com/settings/installations/<installation id>` (the id is in the error), save, and run the script again. The script waits for the authorisation, links the repo and prints `github_repository_id`.
   3. Put that value in `terraform.tfvars` and `terraform apply` again: this creates the `playground-deployer` service account and the trigger. Push to `main`: `cloudbuild.yaml` builds, tests and deploys the real image (the service starts on a placeholder image).
   4. Optional: set `enable_github_connection_setup = false` and apply to drop the service agent's Secret Manager grant.
7. Re-verify Cloudflare's IP ranges (`modules/edge`), watch Cloud Armor's preview logs for real traffic matching the origin rules, then set `enforce_origin_restriction = true`.

## Private Service Connect: port mapping (legacy endpoint retired)

`modules/atlas` creates a **port-mapped** endpoint: one address and one forwarding rule, with connection hostnames `-psc-N`. Atlas disables the old per-node ("legacy", 50 forwarding rules, `-pl-N`) architecture on 2027-04-30, so it is not used here. The app's connection strings use the entry whose `endpoint_id` is our forwarding rule (see `terraform output atlas_private_endpoint_candidates`).

The Atlas private endpoint *service* for a region is shared per Atlas project. If another application's endpoint group is registered on the same project and region, **never let Terraform destroy the shared legacy service**. It must be removed from the state without being destroyed:

```bash
terraform state rm module.atlas.mongodbatlas_privatelink_endpoint.this
```

**Upgrade steps (provider 1.x to 2.x plus port mapping):**
1. Back up the state: `gcloud storage cp gs://<project>-tfstate/mongodb-playground/default.tfstate ./tfstate.backup`.
2. `terraform init -upgrade -backend-config="bucket=<project>-tfstate"` (moves the `mongodbatlas` provider to 2.x and updates `.terraform.lock.hcl`; commit it).
3. If a legacy `mongodbatlas_privatelink_endpoint.this` is in the state, `terraform state rm` it as above, and also remove its legacy endpoint service from the state (`terraform state rm module.atlas.mongodbatlas_privatelink_endpoint_service.this`) if it is no longer in Atlas.
4. `terraform plan -out=upgrade.tfplan`: expect the port-mapped endpoint, address, forwarding rule and endpoint service added, the 50 legacy addresses and forwarding rules destroyed, and the URI secrets replaced (`pending` until the endpoint is registered). Stop if the plan shows the cluster or a database user being replaced.
5. `terraform apply upgrade.tfplan`, then `terraform plan` and `apply` once more so the secrets get the real `-psc-0` connection string, then redeploy the service and check `/readyz`.

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
- Atlas Search on the M10: `GET /api/v1/limits` should report `searchSupported: true`.
- The Google-managed certificate can take a while to provision behind a proxied Cloudflare record.
- Turnstile is not wired in yet (the UI does not send tokens); the abuse controls are Cloudflare rate limits and the app's own limits.
