# Security Policy

## Supported versions

There is no formal release support matrix at this time. Security reports should
identify the affected release or commit. The default main branch is the current
development line.

## Reporting a vulnerability

Please do not disclose security vulnerabilities in a public issue or pull
request.

Use GitHub's private vulnerability reporting option under the repository's
Security tab when it is available. If private reporting is not enabled or is
unavailable to you, contact the maintainer, [@Frank1o3](https://github.com/Frank1o3),
and request a private reporting channel before sending exploit details.

Reports are especially important for issues involving:

- APK download, URL validation, redirects, or archive handling;
- APK signing-certificate or signature verification;
- session cookies, credentials, local profile storage, or secret logging;
- filesystem writes, extraction paths, or privilege boundaries;
- dependency or build supply-chain compromise.

Include the affected commit or release, the impact, and concise reproduction
steps. Share proof-of-concept material only through a private channel. Redact
cookies, tokens, account identifiers, and personal data.

Please do not send real account cookies or credentials as part of a report.
There is no bug bounty or guaranteed response-time commitment.
