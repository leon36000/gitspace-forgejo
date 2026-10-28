# CONTINUATION-264 — webhooks Forgejo émis, receiver attesté et reprise de batch

Date : 27 octobre 2026 (America/Montreal). Worktree `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, arbre local fortement modifié. Aucun commit, publication, service externe, production ou code Forgejo/GitSpace copié.

## Contrat de licence Cargo relu

`cargo metadata --offline --locked --no-deps --format-version 1` confirme `agent-forge-core 0.1.0` avec `license=null`, `license_file=null`, `publish=[]`, `metadata.agent-forge.licensing.decision=no_license_selected`, `redistribution_rights=not_assessed` et `spdx_package_license_declared=NOASSERTION`. `scripts/verify_cargo_license_state.py` renvoie `metadata_policy_verified`; ses 12 tests passent. Ce readback rend l’absence de choix explicite sans sélectionner une licence et ne statue pas sur les droits GitSpace, Forgejo ou des dépendances.

## Désynchronisation des schémas découverte puis corrigée

Avant le run live, la suite `tests.test_forgejo_emitted_webhook_e2e` a échoué sur 36 cas (16 assertions, 20 erreurs). Le binaire Rust courant déclare `RECONCILE_SINGLE_SCHEMA_VERSION=12` et `RECONCILE_BATCH_SCHEMA_VERSION=13`, tandis que les trois validateurs Python concernés attendaient encore single 11 et batch 12. Les fixtures de batch présentes utilisent déjà v13. J’ai réaligné les validations dans `run_forgejo_live_webhook_reconcile_emitted.py`, `run_forgejo_native_branch_delete_webhook_reconcile.py` et `run_forgejo_native_push_webhook_reconcile.py`; aucune version Rust ni donnée de fixture n’a été modifiée. La sortie brute de cette première exécution n’a pas été sauvegardée; le constat et la commande sont consignés ici.

Après correction, `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v` passe **131/131**, et l’analyse AST des trois scripts et du module de tests réussit. Les fichiers source et le journal complet de ce dernier run sont dans `source-sha256.txt` et `python-tests.log`.

## Run Forgejo local après activation de l’attestation receiver

Forgejo `17.0.0+gitea-1.22.0`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, écoute temporairement sur `127.0.0.1:3130`. Le commit exact ayant construit le binaire reste inconnu. Cargo/Rust compile les deux binaires du receiver et du reconciler en mode offline/locked sous Bubblewrap 0.9.0; détails et empreintes figurent dans `provenance.json` et `cargo-build.log`.

Le runner actuel crée une graine Ed25519 privée dans le répertoire d’état mode `0700`, en dérive la clé publique, démarre le receiver signé et passe la politique de clé approuvée au reconciler. Les fichiers de preuve ne contiennent ni la graine ni le secret HMAC. La sortie expose le contrat `receiver_attestation_verification=requires_external_trusted_key_policy; required_before_credential_for_in_scope_delivery`, pas un champ booléen par livraison; le succès de ces readbacks est à interpréter avec le chemin exécuté et les tests de lecture du credential de CONTINUATION-261, pas comme une attestation universelle du lanceur ou du writer local.

Forgejo a émis **7** livraisons réelles pour le dépôt privé `forge-admin/agent-forge-core` : `issues/opened`, `issues/closed`, deux `issues/edited`, `pull_request/opened`, `issue_comment/created` et `pull_request/closed`. Elles apparaissent dans `webhook-events.json`, séquences 1 à 7. Le reader `forge-reader` est non-admin, pull-only et sans droit push. La réconciliation PR en batch est v13, `current_state_read`, avec 3 résultats liés aux IDs demandés et 2 lectures amont pour les deux ressources. Le credential invalide donne `inconclusive/upstream_unauthorized`. La PR d’essai est fermée sans fusion; ce lot n’a pas modifié de code candidat.

Le receiver est ensuite tué par SIGKILL et redémarré sur le même journal. La redélivraison locale de la première entrée retourne HTTP 202, `duplicate=true`, UUID identique et séquence 1. Avant/après : 7 enregistrements, 56 432 octets, SHA-256 identique `5b3c4784d24077736ee42973dcc7c43c165d02ddd11f695ab2707d78c51eb9e9`. Le payload est re-sérialisé depuis l’objet JSON journalisé et re-signé; le corps HTTP original n’est pas conservé et ce n’est pas une répétition native octet pour octet. Forgejo, hook et receiver s’arrêtent, les listeners attendus sont absents et le hook est relu inactif (`port-cleanup-readback.json`).

## Reproduction et limites

Commandes exactes : `commands.txt`. Runner : `scripts/run_forgejo_live_webhook_reconcile_emitted.py --batch-pull-request-events`. État local privé conservé à `/tmp/forgejo-attested-emitted-264-state`; ne pas supprimer. Répertoire de preuves privé mode `0700`; manifestes locaux et globaux sont append-only et leurs préfixes sont comparés par `manifest-prefix-before.json` et `verify_manifests.py`.

Portée `n=1`, instance et données synthétiques locales; aucun LLM, AgentOS, RAG, mémoire ou gain comparatif. Le code candidat n’est pas exécuté. Forgejo, receiver et CLI partagent l’UID hôte; cette composition ne prouve pas la séparation agent/vérificateur/autorité de fusion ni l’exclusion de writers concurrents. La graine privée, le secret HMAC et l’état SQLite ne quittent pas le bac à sable. La licence du produit demeure non choisie et les droits tiers non évalués. Le projet n’est pas achevé.

Prochaine tranche : tenter une seule composition MISSION-012 avec le superviseur hôte et namespace réseau privé, `local-test-adapter` via MCP stdio, attestation receiver, barrière gate et readback post-fusion. Elle doit réunir les preuves actuellement séparées de webhook signé, exécution isolée, gate et fusion; si l’admission système refuse, conserver le diagnostic précis sans relancer de scénario déjà prouvé.
