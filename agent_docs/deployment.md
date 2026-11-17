# Secure Deployment Guide & Systemd Integration (CRA) — `dtnpm`

This guide presents production deployment recommendations in accordance with Cyber Resilience Act (CRA) requirements: least privilege, `systemd` sandboxing, and protected secrets management.

---

## 1. Deployment Security Principles

1. **Dedicated Unprivileged User**:
   - `dtnpm` must NEVER be run as `root`.
   - Create a dedicated system user and group:
     ```bash
     sudo adduser --system --group --no-create-home --home /var/lib/dtnpm dtnpm
     ```
2. **Strict Permissions on Directories and Files**:
   - Configuration files (`config.toml`, `config.yaml`) contain secrets (passwords, BPSec keys, access tokens) and must be protected with `0600` permissions:
     ```bash
     sudo chmod 0600 /etc/dtnpm/config.toml
     sudo chown dtnpm:dtnpm /etc/dtnpm/config.toml
     ```
3. **Passing Secrets Without Leaking in `/proc`**:
   - Do not pass tokens or passwords via plaintext CLI arguments.
   - Use protected token files (`--token-file /etc/dtnpm/token.secret`) or environment variables loaded from protected files (`EnvironmentFile`).

---

## 2. Hardened Systemd Units

The repository provides two hardened template units:
- [dtnpm.service](../dtnpm.service): Standard BPA daemon and Web server service.
- [dtn-hdy-ham.service](../dtn-hdy-ham.service): Amateur radio service with radio CLA management.

### Excerpt of Recommended CRA Hardening Directives:
```ini
[Unit]
Description=DTN Postmaster Standalone Daemon
After=network.target

[Service]
Type=simple
User=dtnpm
Group=dtnpm
ExecStart=/usr/local/bin/dtnpm -c /etc/dtnpm/config.toml
Restart=on-failure
RestartSec=5

# Filesystem and process isolation (CRA / Least Privilege)
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectControlGroups=true
PrivateTmp=true
NoNewPrivileges=true
RestrictSUIDSGID=true
LockPersonality=true

# Automatically managed directories with strict permissions
StateDirectory=dtnpm
StateDirectoryMode=0700
ConfigurationDirectory=dtnpm
ConfigurationDirectoryMode=0750
RuntimeDirectory=dtnpm
RuntimeDirectoryMode=0750

[Install]
WantedBy=multi-user.target
```

---

## 3. Deploying Satellite Utilities (`hdy-stats`, `dtntrigger`)

- When `hdy-stats` subscribes to journald logs, add the `dtnpm` user to the `systemd-journal` group:
  ```bash
  sudo usermod -aG systemd-journal dtnpm
  ```
- If `dtntrigger` must launch external scripts, restrict write permissions on those scripts to the system administrator to prevent script tampering by third parties.
