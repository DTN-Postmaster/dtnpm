# Guide de Déploiement Sécurisé & Intégration Systemd (CRA) — `dtnpm`

Ce guide présente les recommandations de déploiement en environnement de production conformément aux exigences du Cyber Resilience Act (CRA) : moindre privilège, sandboxing `systemd` et gestion protégée des secrets.

---

## 1. Principes de Sécurité au Déploiement

1. **Utilisateur non privilégié dédié** :
   - `dtnpm` ne doit JAMAIS être exécuté en tant que `root`.
   - Créer un compte système et un groupe dédiés :
     ```bash
     sudo adduser --system --group --no-create-home --home /var/lib/dtnpm dtnpm
     ```
2. **Permissions strictes sur les répertoires et fichiers** :
   - Les fichiers de configuration (`config.toml`, `config.yaml`) contiennent des secrets (mots de passe, clés BPSec, jetons d'accès) et doivent être protégés avec les droits `0600` :
     ```bash
     sudo chmod 0600 /etc/dtnpm/config.toml
     sudo chown dtnpm:dtnpm /etc/dtnpm/config.toml
     ```
3. **Passage des secrets sans fuite dans `/proc`** :
   - Ne pas passer de jetons ou mots de passe via des arguments CLI en clair.
   - Utiliser des fichiers de jetons protégés (`--token-file /etc/dtnpm/token.secret`) ou des variables d'environnement chargées depuis des fichiers protégés (`EnvironmentFile`).

---

## 2. Unités Systemd Durcies

Le dépôt fournit deux unités modèles durcies :
- [dtnpm.service](../dtnpm.service) : Service standard du démon BPA et serveur Web.
- [dtn-hdy-ham.service](../dtn-hdy-ham.service) : Service radioamateur avec gestion de CLAs radio.

### Extrait des directives de durcissement CRA recommandées :
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

# Isolation du système de fichiers et des processus (CRA / Moindre Privilège)
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectControlGroups=true
PrivateTmp=true
NoNewPrivileges=true
RestrictSUIDSGID=true
LockPersonality=true

# Répertoires gérés automatiquement avec permissions strictes
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

## 3. Déploiement d'Utilitaires Satellites (`hdy-stats`, `dtntrigger`)

- Lorsque `hdy-stats` s'abonne aux logs de journald, ajouter l'utilisateur `dtnpm` au groupe `systemd-journal` :
  ```bash
  sudo usermod -aG systemd-journal dtnpm
  ```
- Si `dtntrigger` doit lancer des scripts externes, restreindre les droits d'écriture sur ces scripts à l'administrateur système pour éviter toute altération de script par un tiers.
