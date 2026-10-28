# CARGO-DEPENDENCY-LICENSE-INVENTORY-001 — instantané local des dépendances Cargo

Date : 25 octobre 2026 (America/Montreal). Base de travail observée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec modifications préexistantes non commitées. Cargo 1.95.0, rustc 1.95.0, Python 3.12.3, Linux arm64.

## Résultat observé

Sur le `Cargo.lock` dont le SHA-256 est `bf599559dd11ff21ecc4754aa8a693966b4a9ca401153d7d7caa1f18d8995fcc`, `cargo metadata --offline --locked --format-version 1` a retourné 24 paquets de registre et 6 dépendances directes du paquet de travail. Les métadonnées brutes contiennent une expression `license` pour 24/24 paquets; le collecteur a trouvé au moins un fichier de licence conventionnel à la racine du cache local pour 24/24, avec 22 empreintes SHA-256 distinctes. Les versions, expressions non normalisées, fichiers, tailles et empreintes par paquet sont conservés dans [dependency-licenses.json](dependency-licenses.json); les agrégats sont dans [summary.json](summary.json).

Les expressions brutes observées sont : 18 `MIT OR Apache-2.0`, 2 `MIT`, 1 `BSD-3-Clause`, 1 `Unlicense OR MIT`, 1 `(MIT OR Apache-2.0) AND Unicode-3.0`, et 1 valeur brute `MIT/Apache-2.0` (`version_check 0.9.5`) conservée telle quelle. Ce dernier format n’a pas été normalisé dans l’inventaire.

Le contrôle local `scripts/verify_cargo_license_state.py` retourne `status=verified` pour le paquet `agent-forge-core` : `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`. Son readback est archivé dans [product-license-state.json](product-license-state.json).

## Méthode et portée

Le collecteur relit les métadonnées Cargo en mode `--offline --locked`, filtre les sources `registry+`, conserve leurs déclarations brutes et calcule l’empreinte du manifeste de chaque crate. Il relève à la racine les fichiers dont le nom correspond aux motifs `LICENSE`, `LICENCE`, `COPYING` ou `NOTICE`, et inclut le chemin exact si Cargo déclare `license_file`. Il n’enregistre que noms, tailles et hashes des textes. Commandes et versions : [commands.txt](commands.txt), [toolchain.txt](toolchain.txt).

C’est un inventaire technique borné au lockfile et au cache Cargo présents lors du run. Il ne lit ni n’interprète les textes complets, ne normalise ni ne vérifie la validité des déclarations SPDX, ne couvre pas toutes les notices, dépendances hors Cargo, code généré ou éléments non verrouillés, et n’évalue ni compatibilité juridique ni obligations de redistribution. Les fichiers détectés et leurs hashes ne constituent pas une validation des droits. L’état de licence du produit reste sans choix et la redistribution reste non évaluée. Aucun composant upstream GitSpace ou Forgejo n’a été copié; aucun droit nouveau n’est accordé par ce snapshot.

## Vérification reproductible

Le collecteur termine avec code 0 si chaque paquet de registre a une métadonnée brute et au moins un fichier conventionnel détecté. Le contrôle produit de licence s’exécute séparément. Le manifeste local `SHA256SUMS` est contrôlé depuis son dossier avec `(cd evidence/cargo-dependency-license-inventory-001 && sha256sum -c SHA256SUMS)`; l’index append-only du dépôt et celui du projet parent enregistrent les chemins actuels et leurs hashes dans `global-current-entries.txt`. Une première invocation depuis la racine a échoué uniquement parce que les chemins du manifeste sont relatifs à son dossier; le contrôle relancé depuis le bon dossier passe. Le premier audit intégral a détecté un hash courant périmé pour le manifeste local après sa régénération; une nouvelle ligne a été ajoutée aux index sans réécrire leurs anciennes entrées. L’audit final couvre 5 317 chemins uniques dans l’index du worktree (7 lignes historiques non hashables) et 805 dans celui du parent (2 lignes historiques non hashables), sans chemin manquant ni dernier hash divergent.
