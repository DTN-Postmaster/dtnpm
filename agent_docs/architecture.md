# Architecture Système & Flux de Données — `dtnpm`

Ce document détaille l'architecture modulaire de `dtnpm`, sa passerelle de sécurité gRPC, ainsi que les flux de données entre le démon BPA et les utilitaires satellites.

---

## 1. Vue d'Ensemble & Architecture Réseau

`dtnpm` s'interface avec le moteur BPA `hardy` via deux mécanismes complémentaires :
1. **Passerelle d'enregistrement gRPC sécurisée** (`GrpcRegistrationGate`) :
   - Expose une interface gRPC sur port local ou via socket UNIX dédiée (`/run/dtnpm/dtnpm.sock`).
   - Assure l'absence de course TOCTOU en liant le socket immédiatement au démarrage et en bloquant les enregistrements non autorisés tant que l'authentification et l'initialisation ne sont pas achevées.
2. **Couche Web & Ingestion de Bundles** :
   - `WebBundleInjector` enregistre un pseudo-CLA en interne pour injecter directement les bundles en provenance des WebModules (`mod-chat`, `mod-files`, `mod-senml`) dans le pipeline BPA.

```mermaid
graph TD
    subgraph Clients["Clients CLI & Externes"]
        CLI["dtnprint / dtnsend / dtnping / ..."]
        Basket["dtnbasket-cli"]
    end

    subgraph Daemon["dtnpm Daemon (Serveur Hardy BPA)"]
        Gate["GrpcRegistrationGate / Socket UNIX"]
        BPA["Hardy BPA Core Pipeline"]
        WebCore["dtnpm-web-core (Axum / IAM)"]
        Modules["Modules: Chat / Files / Senml"]
        CLAs["CLAs: LTP / HQFBP / Mercury / TCPCL"]
    end

    CLI -->|gRPC sécurisé / Token| Gate
    Basket -->|gRPC sécurisé / Token| Gate
    Gate --> BPA
    WebCore --> Modules
    Modules -->|WebBundleInjector| BPA
    BPA <--> CLAs
```

---

## 2. Binaires CLI & Spécificités d'Ingénierie

### 1. Le Démon Principal (`dtnpm`)
- Intègre le BPA `hardy`, l'authentification locale PBAC, le gestionnaire de CLA et le serveur Web.
- Gère la configuration par défaut avec permissions Unix `0600` via l'assistant `wizard`.

### 2. Le Récepteur d'Impression (`dtnprint`)
- S'enregistre en tant que service auprès du BPA.
- Reçoit les bundles CBOR bruts, vérifie l'intégrité BPSec (BIB) et écrit le contenu atomiquement sur `stdout`.

### 3. L'Émetteur (`dtnsend`)
- Enregistre dynamiquement un CLA éphémère auprès du BPA pour injecter des bundles sans restriction d'adresse source.
- Prend en charge la signature BPSec (HMAC-SHA256 ou Ed25519) et le mode simulation hors-ligne (`-D / --dry-run`).

### 4. L'Outil d'Interrogation du Stockage (`dtnquery`)
- Accède hors-ligne directement aux métadonnées SQLite (`metadata.db`) ou PostgreSQL.
- Utilise des requêtes avec pushdown SQL (`LIKE ... ESCAPE` et `LIMIT`) pour éviter de charger en mémoire l'intégralité des bundles lors du filtrage par adresse ou du calcul des statistiques globales.

### 5. Le Déclencheur Événementiel (`dtntrigger`)
- Réceptionne les bundles sur un endpoint dédié, vérifie la signature BPSec, écrit le payload dans un fichier temporaire sécurisé (`NamedTempFile`), et exécute la commande configurée sans passer par un interpréteur shell non contrôlé.

### 6. L'Outil de Diagnostic (`dtnping`)
- Émet des requêtes d'écho DTN avec horodatage et calcule les temps d'aller-retour (RTT).
- Borne sa table de suivi à 10 000 entrées pour prévenir toute fuite mémoire.

### 7. Le Récepteur de Fichiers (`dtnfiles`)
- Sauvegarde les fichiers reçus après contrôle de quota préalable, assainissement strict du nom de fichier (128 caractères alphanumériques et tirets) et écriture asynchrone non bloquante.

### 8. L'Encapsuleur BIBE (`dtnforward` / `dtnbib`)
- Encapsule les bundles entrants dans des bundles externes (BIBE - RFC 5050 / RFC 9171).
- Détermine avec précision si la destination est locale via comparaison des autorités de nœuds (`NodeId`).

### 9. Le Collecteur de Métriques (`hdy-stats`)
- Surveille l'état du démon via journald ou lecture de fichier de log.
- Restreint l'autorité du destinataire des rapports (`report_to`) au nœud émetteur sauf autorisation explicite via `--allow-third-party-report-to`.
- Valide le nom de l'unité journald via une expression régulière stricte pour prévenir toute injection d'options.
