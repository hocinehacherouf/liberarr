# Security policy

## Supported versions

Only the latest release gets security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through [GitHub private vulnerability reporting](https://github.com/hocinehacherouf/liberarr/security/advisories/new), not in a public issue.

Include the version (`liberarr --version`), your platform, and the steps to reproduce. Never attach real recordings, transcripts, `.env` files, keys, tokens or device serials: use synthetic data.

Liberarr is maintained by one person in their spare time. You should get an answer within 7 days. A fix is released as soon as possible, and the advisory is published with the fix.

## Scope

In scope: the `liberarr` binary, its local web UI and API, its handling of secrets in `LIBERARR_HOME`, and the release workflow.

Out of scope: vulnerabilities in the Plaud device or cloud, and in third-party AI providers. Report those to their vendors.
