# Agent Guide — `dtnpm`

## 🎯 Mission
Fournir le démon autonome BPA `dtnpm`, l'assistant de configuration interactive (`wizard`), les utilitaires en ligne de commande DTN (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `hdy-stats`), ainsi que l'orchestration des couches de convergence (`dtnpm-cla-ltp-kiss`, `dtnpm-cla-hqfbp-kiss`, `dtnpm-cla-mercury-arq`), des modules WebUI (`dtnpm-web-core`, `dtnpm-mod-*`) et de l'application Basket (`dtnpm-app-basket`), en interaction directe avec l'implémentation DTN BPv7 de `hardy`.

---

## 🛡️ Référentiels de Sécurité et Exigences Normatives

Chaque agent et développeur intervenant sur ce dépôt DOIT intégrer dès la conception les exigences suivantes :

### 1. Cyber Resilience Act (CRA - Règlement UE 2024/2847)
- **Sécurité par défaut (Secure-by-Default)** : Toutes les fonctionnalités doivent être configurées dans l'état le plus protecteur par défaut.
- **Moindre privilège et isolation système** :
  - Permissions strictes `0600` / `0700` sur les fichiers de configuration contenant des secrets (`config.toml`, keystores, jetons).
  - Services systemd (`dtnpm.service`, `dtn-hdy-ham.service`) exécutés sous utilisateur non-privilégié (`User=dtnpm`, `Group=dtnpm`) avec sandboxing actif (`ProtectSystem=strict`, `ProtectHome=true`, `NoNewPrivileges=true`, `RestrictSUIDSGID=true`, `StateDirectory=dtnpm`, `ConfigurationDirectory=dtnpm`).
- **Secret Zero dans les lignes de commande** :
  - Ne JAMAIS forcer le passage de jetons ou mots de passe via des arguments CLI visibles dans `/proc/<pid>/cmdline`.
  - Utiliser systématiquement `--token-file <PATH>` ou les variables d'environnement (`DTN_HDY_TOKEN_FILE`, `DTN_HDY_TOKEN`), et `--password-stdin` pour les commandes d'administration des utilisateurs.
- **Supply-Chain & VEX (Vulnerability Exploitability eXchange)** :
  - Tout avis de sécurité `cargo-audit` / `cargo-deny` doit être immédiatement résolu ou formellement justifié avec document VEX dans `.cargo/audit.toml` et `deny.toml` (ex. `RUSTSEC-2026-0459` en `not_affected` car l'extension SQLite session est inutilisée).
  - Génération automatique de SBOM CycloneDX (`sbom.cdx.json`) et sommes de contrôle cryptographiques (`SHA256SUMS`) lors des releases.
  - Épinglage cryptographique des dépendances CI : toutes les actions GitHub doivent être épinglées par SHA-1 complet de 40 caractères avec bloc de permissions minimal `permissions: contents: read`.
- **Politique de divulgation des vulnérabilités** : Consignée et tenue à jour dans [SECURITY.md](SECURITY.md).

### 2. OWASP (Top 10 / ASVS / API Security)
- **A01 Broken Access Control & IAM** :
  - La passerelle gRPC locale utilise une barrière d'enregistrement sécurisée (`GrpcRegistrationGate` / `GuardedBpaRegistration`) évitant toute fenêtre de course TOCTOU au démarrage.
  - Contrôle d'accès PBAC (Policy-Based Access Control) via moteur Rego et isolation stricte des rôles (`operator`, `admin`).
- **A02 Cryptographic Failures** :
  - Utilisation systématique de SHA-256 pour les condensats (bannissement de SHA-1 pour l'intégrité).
  - Vérification BPSec (RFC 9172) stricte par défaut : rejet des bundles non signés ou à clé inconnue.
- **A03 Injection & Assainissement** :
  - Assainissement strict des chaînes d'entrée, des identifiants d'endpoints (EID), des indicatifs radioamateurs (`[A-Z0-9/-]+`) et des noms de fichiers (suppression des traversées de chemin `..`, caractères dangereux et limitations de longueur).
  - Exécution des sous-processus sans shell (`tokio::process::Command::kill_on_drop(true)` avec timeout borné et arguments vectorisés, séparateur `--`).
- **A05 Security Misconfiguration & Opt-in explicite** :
  - L'activation d'un comportement non sécurisé ou d'un mode dégradé (ex. liens réseau non signés, désactivation d'authentification gRPC en labo, indicatif RF générique `N0CALL`, autorisation d'adresses privées) n'est possible QUE si l'utilisateur le demande EXPRESSÉMENT (ex. `--allow-unauthenticated-udp`, `--allow-unidentified-rf`, `--allow-third-party-report-to`).

---

## 🦀 Bonnes Pratiques d'Ingénierie Rust

1. **Règle Zéro Panic / Zéro Unwrap** :
   - Le code de production (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`) DOIT être 100% exempt de `.unwrap()`, `.expect()` et de macro `panic!`.
   - Utiliser la propagation d'erreurs `?`, `.ok_or_else()`, ou des types d'erreurs `thiserror` / `anyhow`.
   - Linter strict imposé en CI et en pré-commit : `-D warnings -D clippy::unwrap_used -D clippy::panic`.
2. **Processus d'Architecture et Traçabilité (ADR)** :
   - Toute décision architecturale, évolution de protocole ou ajout de composant DOIT être documentée dans le dossier `adr/` sous forme d'un Architectural Decision Record numéroté, et indexée dans [adr/README.md](adr/README.md).
3. **Robustesse et Bornage des Ressources** :
   - Toujours borner les allocations mémoire, les lectures de flux (`.take()`), les sémaphores de processus concurrents et les tables de suivi des pairs.
   - Prévenir les attaques par déni de service (DoS) par saturation mémoire (OOM) ou CPU.

---

## ⚡ Commandes Critiques de Vérification

Avant TOUTE finalisation de tâche ou proposition de commit, exécuter obligatoirement :

- **Formatage** : `cargo fmt --all -- --check`
- **Linter de sécurité & qualité** : `cargo clippy --offline --workspace --lib --bins -- -D warnings -D clippy::unwrap_used -D clippy::panic`
- **Tests complets** : `cargo test --offline --all-targets`
- **Profils de compilation** :
  - `cargo check --offline --no-default-features --features "amsat"`
  - `cargo check --offline --no-default-features --features "node-headless"`
  - `cargo check --offline --no-default-features --features "webui-remote"`
- **Audit de sécurité complet** : `./scripts/check-security.sh --strict`
- **Couverture de code** : `./scripts/check-coverage.sh --summary-only`
- **Installation des hooks Git locaux** : `./scripts/install-hooks.sh`

---

## 🗺️ Cartographie de l'Organisation & Référentiel des Projets

- `src/` : Démon BPA standalone `dtnpm`, assistant `wizard`, passerelle de sécurité gRPC et binaires CLI dans `src/bin/`.
- `../dtnpm-core` : Modèles de données centraux, client gRPC interne (`create_remote_bpa`), passerelle IAM PBAC, keystore BPSec, ordonnancement contact-plan.
- `../dtnpm-web-core` : Passerelle HTTP/Web Axum, interface `WebModule`, authentification IAM, proxying et composants Preact UI sécurisés.
- `../dtnpm-cla-ltp-kiss` : Couche de convergence LTP (RFC 5326 / RFC 7122) et passerelle UDP/KISS.
- `../dtnpm-cla-hqfbp-kiss` : Couche de convergence radio HQFBP sur KISS TCP/Série/UDP avec correction d'erreur FEC.
- `../dtnpm-cla-mercury-arq` : Couche de convergence radio HF connectée Mercury ARQ.
- `../dtnpm-mod-chat` : Module applicatif WebUI de messagerie instantanée DTN sécurisée par BPSec.
- `../dtnpm-mod-files` : Module applicatif WebUI de partage et de transfert de fichiers DTN avec contrôle de quotas.
- `../dtnpm-mod-senml` : Module applicatif WebUI de collecte et visualisation de télémétrie SenML (RFC 8428).
- `../dtnpm-app-basket` : Démon (`dtnbasket`) et CLI (`dtnbasket-cli`) pour le protocole DTN Basket avec mitigation SSRF.
- `adr/` : Registre des décisions d'architecture (ADRs).
- `agent_docs/` : Documentation d'approfondissement technique pour agents autonomes.
- `examples/` : Configurations types, scripts de notification et démonstrateurs.
- `scripts/` : Scripts de validation de sécurité (`check-security.sh`), couverture (`check-coverage.sh`) et hooks Git.

---

## 📚 Index Documentaire

1. [agent_docs/conventions.md](agent_docs/conventions.md) : Conventions de nommage, flux I/O, gestion du temps et Zero-Unwrap.
2. [agent_docs/testing_guidelines.md](agent_docs/testing_guidelines.md) : Stratégie de tests d'intégration, mocks hors-ligne et couverture.
3. [agent_docs/architecture.md](agent_docs/architecture.md) : Architecture système, passerelle gRPC sécurisée et flux de données.
4. [agent_docs/deployment.md](agent_docs/deployment.md) : Guide de déploiement sécurisé avec systemd (sandboxing CRA).
5. [adr/README.md](adr/README.md) : Index de l'ensemble des Architectural Decision Records.
6. [SECURITY.md](SECURITY.md) : Politique de sécurité et processus de divulgation des vulnérabilités.
