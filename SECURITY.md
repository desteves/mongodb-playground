# Security policy

MongoDB Playground runs untrusted queries against a shared database, so security reports are welcome.

## Reporting a vulnerability

Please report privately through GitHub's "Report a vulnerability" (Security tab -> Advisories) rather than a
public issue. Include what you did, what you expected, and what happened. Expect an acknowledgement within a
few days.

## Scope

In scope: escaping a playground's throwaway database, reading another user's data or shares, bypassing the
operator allowlist (`$where`, `$function`, `$out`, `$merge`, cross-database references), resource exhaustion that
the limits should stop, and secrets exposure.

Out of scope: volumetric denial of service, findings that need physical access, and issues in third-party
services (Atlas, Cloudflare, Google Cloud) themselves.

## Design notes

Queries run in a per-run `pg_*` database that is always dropped (with a sweep job as backup). Share links are
random UUIDs, expire after 90 days without a view, and the app warns users not to share sensitive data. Usage
statistics hold metadata only: no query text, no documents, no raw IP addresses. See `docs/MASTER_PLAN.md`
for the full security model.
