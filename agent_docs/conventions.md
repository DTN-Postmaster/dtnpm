# Conventions de Développement — `dtnpm`

Ce document formalise les conventions, standards de codage et exigences de robustesse applicables au projet `dtnpm`.

---

## 1. Standards de Code & Zéro Panic / Zéro Unwrap

- **Règle absolue** : Aucun appel à `.unwrap()`, `.expect()` ni invocation de la macro `panic!()` n'est autorisé dans le code de production (`--lib`, `--bins`).
- **Gestion des erreurs** :
  - Utiliser l'opérateur `?` pour propager les erreurs.
  - Transformer les options via `.ok_or_else(|| ...)` ou `.ok_or(...)`.
  - Pour les mutex et verrous partagés, gérer le cas d'un verrou empoisonné de manière sûre : `lock().map_err(|e| ...)` ou `unwrap_or_else(|e| e.into_inner())`.
- **Validation statique** : Le linter strict doit être exécuté et passer sans aucun avertissement :
  ```bash
  cargo clippy --workspace --lib --bins -- -D warnings -D clippy::unwrap_used -D clippy::panic
  cargo fmt --all -- --check
  ```

---

## 2. Arguments en Ligne de Commande & Gestion des Secrets (CRA)

- **Parcours CLI `clap`** : Toutes les commandes utilisent l'attribut `derive` de `clap`.
- **Gestion sécurisée des secrets** :
  - Conformément aux exigences du Cyber Resilience Act (CRA), ne JAMAIS forcer l'utilisateur à passer des mots de passe ou des jetons d'authentification en clair dans la ligne de commande.
  - Privilégier systématiquement `--token-file <PATH>` (avec support de la variable d'environnement `DTN_HDY_TOKEN_FILE`).
  - L'option dépréciée `--token` doit afficher un avertissement de sécurité sur `stderr` incitant à migrer vers `--token-file`.
  - Pour les commandes de création/modification d'utilisateurs (`user add`, `user passwd`), fournir l'option `--password-stdin` pour injecter le secret depuis un canal sécurisé.
- **Résolution de port** :
  - Port gRPC local par défaut : `50051`.
  - Port Web / HTTP par défaut : `3000`.
  - Toujours respecter les variables d'environnement prioritaires `HARDY_GRPC_PORT` ou `DTN_WEB_PORT`.

---

## 3. Séparation des Flux de Sortie

- **`stdout`** : Réservé exclusivement aux sorties fonctionnelles et données consommables par des scripts en pipeline (payloads décodés, identifiants de bundles, CBOR sérialisé, statistiques JSON, hachages de digest, métriques de ping).
- **`stderr`** : Réservé exclusivement à la journalisation, aux avertissements de sécurité, aux traces d'exécution et aux messages d'erreur.
- **Écritures atomiques** : Verrouiller `std::io::stdout().lock()` lors de l'écriture de blocs multi-lignes pour éviter tout entrelacement lors d'exécutions concurrentes.

---

## 4. Dates et Horodatages

- Utiliser systématiquement la bibliothèque `time` (notamment `time::OffsetDateTime`).
- Formater les dates en chaînes conformes à `Rfc3339` (`time::format_description::well_known::Rfc3339`).

---

## 5. Dérogations de Sécurité Explicites (Opt-in)

- Tout comportement permissif (ex. acceptation de trafic non signé, tolérance de pairs non identifiés) doit exiger un drapeau CLI explicite ou une option dédiée dans le fichier de configuration (ex. `--allow-unauthenticated-udp`, `--allow-unidentified-rf`, `--allow-third-party-report-to`).
- Il est strictement interdit d'abaisser les niveaux de sécurité par défaut sans accord explicite de l'utilisateur.

---

## 6. Traçabilité Architecturale (ADR)

- Toute modification de structure, de modèle de données ou de protocole de sécurité doit faire l'objet d'un nouvel enregistrement ADR dans `adr/` et être référencée dans `adr/README.md`.
