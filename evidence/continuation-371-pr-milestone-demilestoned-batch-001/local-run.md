# CONTINUATION-371 — désaffectation d’un milestone PR à travers le batch CLI

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste fortement modifié; aucun reset, staging, commit, fetch/push ou suppression n’a été effectué. Ce lot n’a modifié aucun code runtime ni dépendance. Voir `environment.txt` pour l’état condensé et `source-sha256.json` pour les hashes de sources.

## Contrat exercé

Le test `actual_reconcile_cli_process_projects_current_unassigned_pull_request_milestone_for_batch` envoie deux payloads signés avec le secret de fixture, pour la PR 50 et dans cet ordre : `milestoned` avec le milestone historique ID 13, puis `demilestoned` avec `pull_request.milestone: null`. Le vrai receiver répond HTTP 202 aux deux et journalise les séquences 1 et 2. Après arrêt du receiver, le vrai binaire `forgejo-reconcile-local` reçoit les deux IDs et lit une fixture HTTP API courante qui retourne `milestone: null`.

Le test vérifie que les deux résultats du batch sont `current_state_read`, portent le même état PR courant et projettent `milestone = {observation: unassigned, value: null, source: current_forgejo_resource_api, freshness: current_api_read, trust: untrusted_user_content}`. Un seul GET est partagé entre les deux livraisons; la requête n’a pas de header Authorization. Le journal reste byte-identical et la chaîne du milestone historique n’apparaît pas dans la sortie.

## Test-first et résultats

Avant l’ajout du scénario à `TEST_NAMES`, le runner a refusé `--scenario` avec argparse exit 2 (`runner-allowlist-red.txt`). Après allowlisting, le vrai test ignoré par défaut a passé 1/1 sous Bubblewrap avec namespaces réseau et PID non partagés (`process-e2e.log`). Le contrôle rustfmt intermédiaire a trouvé deux lignes à reformater; le résultat est consigné dans `rustfmt-initial-failure.txt`. Après correction du seul formatage, le même runner a recompilé offline et repassé le scénario 1/1 (`process-e2e-final.log`). Le test reste `#[ignore]` hors de ce runner isolé.

Contrôles finaux : `cargo fmt --all -- --check` passe; `py_compile` des scripts passe; un contrôle final robuste des espaces en fin de ligne sur tout le runner et le fichier de test passe (le premier scanner avait faussement traité des antislashs littéraux de continuation Rust comme espaces; aucun correctif de source n’a été nécessaire); `git diff --check` passe pour les changements suivis. Les 12 tests de `scripts/test_cargo_license_state.py` passent. `scripts/verify_cargo_license_state.py` retourne `metadata_policy_verified`: `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`. Aucun choix de licence n’est fait.

Commandes principales :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_projects_current_unassigned_pull_request_milestone_for_batch
cargo fmt --all -- --check
python3 scripts/test_cargo_license_state.py
python3 scripts/verify_cargo_license_state.py
git diff --check
```

La compilation Python a été faite par `py_compile.compile` avec les fichiers `.pyc` dans `/tmp`, sans ajouter de cache au worktree.

Le journal E2E conserve la ligne complète de commande Bubblewrap, les limites de ressources, la compilation offline et le résultat du test. Aucun appel LLM, AgentOS, Forgejo vivant, courriel, GitLab, transfert réseau, coût externe ou endpoint de production n’a participé à ce lot local.

## Portée et limites

C’est une composition de processus locaux contre événements HMAC synthétiques et une fixture loopback. Elle confirme le comportement du receiver, du journal, du CLI et du cache de lecture API dans ce scénario précis. Elle ne prouve pas que Forgejo émet réellement ces payloads PR dans toutes les versions/configurations, ni que l’état reste inchangé après l’unique GET partagé, ni un parcours LLM/AgentOS, PR→fusion contrôlée, comportement après fusion ou gain comparatif. Elle ne reteste pas la matrice générale de refus ou le parcours déjà prouvé CONT-370.

La prochaine lacune indépendante est l’origine hôte des exigences protégées, du digest et de la commande de vérification : le test d’admission existant vérifie l’égalité par rapport au digest reçu du harnais, mais n’établit pas qui peut autoriser ce digest et ce programme.
