# Security

Report vulnerabilities privately through [GitHub private vulnerability
reporting](https://github.com/kawaiipantsu/aistats/security/advisories/new).
Include the affected commit, the problem, and minimal reproduction steps using
fictional data. Do not attach real chat logs, credentials, configuration files,
private hostnames, or account details.

Use public issues for ordinary bugs with sensitive details removed.
Security fixes target the current `main` branch; this project is distributed
from source without prebuilt releases.

## Protecting local data

Keep provider admin keys and Discord webhook URLs in environment variables or
the private configuration directory. Never commit them. Review reports before
sharing them, including screenshots: pseudonyms can still be recognizable from
activity patterns. Usage reports may include provider or model identifiers.

The repository ignores local configuration, chat logs, keys, databases, salts,
and generated reports. Ignoring a path does not remove previously committed
data. Check your staged diff before every commit, and revoke any credential
that has been exposed. GitHub secret scanning and push protection are enabled.
