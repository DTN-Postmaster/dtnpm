# Guide de Test et d'Assurance Qualité — `dtnpm`

Ce document présente la stratégie de test, la validation hors-ligne, les garde-fous de sécurité et la mesure de couverture pour le projet `dtnpm`.

---

## 1. Tests Unitaires & Intégrité Locale

- **Zéro dépendance réseau en test** : Toutes les suites de tests doivent être exécutables en mode déconnecté (`--offline`).
- **Tests d'encodage/décodage CBOR & EID** : Valider que les identifiants d'endpoints (EID) et les blocs BPv7 (RFC 9171) sont fidèles aux spécifications et protégés contre les dépassements d'entiers ou les allocations non bornées.
- **Vérification BPSec** :
  - Tester les trois modes de politique (`Strict`, `Warn`, `Ignore`).
  - Vérifier que sous `VerifyPolicy::Strict`, les bundles non signés (`Unsigned`), à clé introuvable (`MissingKey`) ou corrompus (`Invalid`) sont systématiquement rejetés.
- **Sanitisation et bornes mémoires** :
  - Valider l'assainissement des noms de fichiers dans `dtnfiles` (protection contre les chemins relatifs, caractères d'échappement, noms trop longs).
  - Valider le filtrage par liste blanche des unités journald dans `hdy-stats` (rejet des tirets initiaux et métacaractères d'injection).
  - Valider les requêtes avec filtre SQL `--addr` et `--limit` dans `dtnquery`.

---

## 2. Tests d'Intégration & Mocks de Services

- Les tests d'intégration simulent les interactions réelles entre composants :
  - **`iam_integration_test.rs`** : Valide le contrôle d'accès PBAC, la gestion des utilisateurs, les sessions, et l'enregistrement de services via la passerelle gRPC et socket UNIX.
  - **`bpsec_chat_files_test.rs`** : Valide la chaîne complète de signature et vérification BPSec pour les modules Chat et Fichiers.
  - **`ham_server_test.rs`** : Valide l'initialisation du serveur dtnpm avec différentes configurations TOML/YAML, la parité des configurations, et la gestion des CLAs.
  - **`bundle_inspector_test.rs`** : Valide l'inspection des bundles en mémoire et sur disque SQLite.
- Les tests asynchrones utilisent `tokio::test` avec des timeouts raisonnables pour éviter tout gel du banc de test.

---

## 3. Couverture de Code (`cargo-llvm-cov`)

Pour surveiller l'absence de régression dans la couverture des tests :
- Rapport console synthétique : `./scripts/check-coverage.sh --summary-only`
- Rapport complet : `./scripts/check-coverage.sh`
- Rapport HTML interactif : `./scripts/check-coverage.sh --html` (disponible sous `target/llvm-cov/html/index.html`)
- Export CI : `./scripts/check-coverage.sh --lcov` (génère `lcov.info`)

> [!NOTE]
> Ne jamais commiter de fichier `lcov.info` ou `sbom.json` généré localement dans l'arbre de travail Git.

---

## 4. Matrice de Validation Pré-Livraison

Avant de finaliser une tâche ou de créer un commit, exécuter obligatoirement :

1. `cargo fmt --all -- --check`
2. `cargo clippy --offline --workspace --lib --bins -- -D warnings -D clippy::unwrap_used -D clippy::panic`
3. `cargo test --offline --all-targets`
4. Profils spécifiques :
   - `cargo check --offline --no-default-features --features "amsat"`
   - `cargo check --offline --no-default-features --features "node-headless"`
   - `cargo check --offline --no-default-features --features "webui-remote"`
5. Audit de sécurité : `./scripts/check-security.sh --strict`
