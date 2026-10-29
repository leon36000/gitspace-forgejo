# CONTINUATION-449 — contrôle des IDs du dépôt base après fusion

Date de travail : 28 octobre 2026. Worktree local fortement modifié, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucune modification de commit, de branche par défaut ou de production.

## Décision et changement

Le DTO Forgejo v17.0.0 `PRBranchInfo` expose séparément `base.repo_id` (int64) et `base.repo.id` (ID du dépôt imbriqué). Le readback qui confirme un merge vérifiait déjà l’identité textuelle de la PR et liait l’ID imbriqué de la PR à celui du readback d’issue; il ignorait le champ numérique `base.repo_id` de la même PR. Il exige maintenant que les deux IDs de `base` soient des entiers signés positifs et égaux. Le contrat `--capabilities` annonce schema 30, API `forgejo-reconcile-capabilities-v30`, les deux chemins et leur règle de liaison.

Cela détecte une réponse incohérente à l’intérieur du même objet PR. Cela ne prouve pas l’authenticité du serveur, ne compare pas à un ID de cible configuré indépendamment, n’élimine pas une course serveur et ne rend pas le POST atomique.

## Test-first et résultats

- `red.log` : avant le correctif, l’unitaire acceptait `base.repo_id=8` avec `base.repo.id=7`; échec attendu sur l’assertion de liaison.
- `unit-green.log` : test unitaire ciblé, 1/1. Les cas négatifs incluent ID divergent, zéro, chaîne et champ absent.
- `process-e2e-final.log` : le vrai CLI est lancé sous Bubblewrap (`--unshare-net`, `--unshare-pid`); une fixture loopback observe le POST appliqué puis renvoie `base.repo_id=18`, `base.repo.id=17`. Le CLI répond `inconclusive` / `merge_outcome_not_confirmed`, consigne une intention commencée et aucune résolution.
- `all-targets-green.log` : `cargo test --locked --offline --all-targets -- --test-threads=1` termine avec 39 groupes, 786 réussites, 0 échec, 210 tests ignorés. Le premier run, `all-targets.log`, a révélé huit anciennes fixtures de readback fusionné qui omettaient désormais `base.repo_id`; elles ont été corrigées pour porter l’ID 7 cohérent, puis la suite complète a passé.
- `capabilities.json` : sortie du binaire réel, schema 30 / API v30 et relation publiée.
- `cargo-metadata.json` : `license=null`, `license_file=null`, `publish=[]`; aucune licence produit n’est sélectionnée, et les droits de redistribution restent non évalués.
- Les contrôles finaux sont listés dans `final-checks.log`; métadonnées d’environnement et de sources amont dans les fichiers adjacents.
- `SHA256SUMS` vérifie 18/18 fichiers du reçu. L’append-manifests confirme les index du worktree et du projet sans chemin manquant, divergent ou non haché; le worktree garde un ancien enregistrement historique sans empreinte, hors de ce reçu.

## Source Forgejo examinée

Le dépôt canonique est `https://codeberg.org/forgejo/forgejo.git`; la branche `forgejo/forgejo` sur GitHub renvoie 404 au moment du contrôle, et le profil GitHub officiel renvoie vers Codeberg. Le site officiel utilise aussi Codeberg pour le code et les versions et nomme `code.forgejo.org` comme miroir. Tag v17.0.0 et commit vérifié `0d791421f85ca13a63b665053d81ea765fbbc571`. Le DTO et les fonctions de conversion exacts, les lignes et les SHA-256 sont dans `upstream-source.txt`. Les quatre fichiers amont cités étaient propres dans le checkout au moment de l’inspection. Aucun code Forgejo n’a été copié ou modifié.

## PC4 et limites opérationnelles

Le connecteur GitLab authentifié lit le projet privé PC4 `root/gitspace-forgejo`, ID 4, et ses refs; le courriel a été consulté à la demande de l’utilisateur, mais les identifiants n’ont pas été utilisés, affichés ou écrits dans les fichiers. Le payload temporaire de la boîte a été vidé. L’état des deux branches, la protection, l’absence de MR et l’absence du rapport TRANSFER-012 dans l’arbre distant sont consignés dans `gitlab-remote-readback.txt`. Le dernier transfert distant, commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb`, a un readback exact des chemins candidats documenté localement dans TRANSFER-011/012. Pipeline 328 : Semgrep et Trivy réussis, Gitleaks échoué sur dix chaînes de 64 caractères sous les noms `forgejo-read.token` et `invalid-forgejo-read.token`. Leur révocation n’est pas établie. Aucune nouvelle écriture GitLab n’a été faite dans ce lot; une décision utilisateur sur conservation ou retrait/révocation reste en attente. Les artefacts CONT-449 sont donc locaux tant que ce point n’est pas réglé.

Ce lot vérifie une condition de refus à partir d’une fixture simulant un payload contradictoire. Il ne réexécute pas le parcours complet de livraison, n’appelle aucun LLM/AgentOS, ne mesure aucun gain comparatif et ne clôt pas la mission.
