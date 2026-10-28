# CONTINUATION-424 — composer Release avec le readback courant sans credential superflu

Date : 28 octobre 2026 (America/Montreal). Worktree : `local/WEBHOOK-005-canonical-links`, HEAD local conservé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Changement

`reconcile_live_release_delivery` relie le record `release` validé au vrai point d’entrée Python `run_reconcile`, puis vérifie un unique résultat courant du CLI lié au delivery, à l’attestation, au dépôt et à l’ID de release. Le credential devient facultatif et vaut `None` par défaut pour cette route publique. Quand il est absent, `run_reconcile` n’ouvre pas de pipe et n’ajoute ni `--host-credential-fd` ni numéro de FD. Un credential hôte explicitement passé conserve le chemin précédent. Le résultat courant reste une projection minimale issue du GET API; le tag et le corps du webhook ne deviennent pas canoniques, et notes/assets ne sont pas exposés.

## Preuve

Le test TDD rouge `credential-red.log` a échoué avant l’implémentation parce que l’interface de composition exigeait encore un argument `credential`. Le test ajouté démarre un petit exécutable de fixture qui capture les arguments du processus, retourne une réponse de fixture, puis vérifie qu’un readback public n’a pas reçu `--host-credential-fd` et reçoit seulement le journal et le delivery ID attendus. La classe de composition passe **6/6**.

Suites Python après correction :

- `test_forgejo_emitted_webhook_e2e.py` : **178/178**.
- `test_forgejo_webhook_native_replay_e2e.py` : **30/30**.
- `test_forgejo_live_org_team_reviewer_readback.py` : **20/20**.
- garde de licence Cargo : **12/12**.
- métadonnées Cargo : `license=null`, `license_file=null`, `publish=[]`, droits de redistribution `not_assessed`, `NOASSERTION`.
- AST Python et `git diff --check` : succès.

Les commandes, versions et hashes sont conservés ici. Aucun code Rust n’a changé; la suite Rust `all-targets` n’a donc pas été relancée pour cette tranche.

## Portée et limites

Le nouveau test exerce le vrai lanceur Python et sa création de processus, mais le binaire enfant est un exécutable de fixture et sa réponse est statique. Ce n’est pas un appel API live ni une émission webhook native Forgejo. La preuve antérieure CONTINUATION-406 reste séparée : elle couvre le vrai CLI Rust et un GET Release anonyme contre un serveur HTTP loopback fixture. Forgejo et Go restent absents du PATH. Aucun modèle, AgentOS, PR ou merge n’est impliqué ici.

Le scope du lot est une fonction de composition réutilisable, pas l’intégration complète à l’entrée principale du runner : `rg -n 'reconcile_live_release_delivery' scripts/run_forgejo_live_webhook_reconcile_emitted.py scripts/run_forgejo_live_webhook_reconcile_e2e.py tests/test_forgejo_emitted_webhook_e2e.py` ne trouve que sa définition et un accès dynamique de test via `getattr`; aucun appel de production n’existe. Le harness d’émission principal ne crée donc pas encore le webhook Release de bout en bout. La matrice de refus et le checkpoint conservent explicitement cette lacune.

Le lot n’affirme pas que toutes les releases d’un dépôt privé sont lisibles sans credential; un credential explicite reste disponible pour les cas qui le nécessitent. La licence produit reste non choisie et les droits sur les sources amont ne sont pas évalués.
