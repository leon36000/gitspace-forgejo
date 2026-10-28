# CONTINUATION-370 — composition PR milestone jusqu’au CLI

Date : 28 octobre 2026. Checkout local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, reset, suppression, fetch/push, GitLab, lecture de courriel, modèle externe, dépense ou changement de production.

## Contrat essayé

Exercer le vrai chemin produit receiver → journal → processus `forgejo-reconcile-local`, pour une notification PR `milestoned`, puis vérifier que le GET API courant détermine seul l’état canonique. La tâche synthétique est la PR 49 : payload webhook HMAC-fixture avec milestone ID 99 périmé; fixture API loopback `/api/v1/repos/forge-admin/agent-forge-core/pulls/49` avec milestone ID 13 et head/base courants.

Le test `actual_reconcile_cli_process_projects_current_pull_request_milestone_from_api` vérifie HTTP 202 et `duplicate=false`, arrête le receiver, rejoue un événement dans le journal, lance le binaire CLI, puis exige : un GET PR, aucun header Authorization, journal byte-identical, verdict `current_state_read`, projection `{id:13,...}` avec provenance API courante, et aucune fuite de la chaîne de milestone périmé dans la sortie.

## Test-first et exécution

Le nouveau cas a d’abord été absent de `TEST_NAMES` dans `scripts/run_webhook_reconcile_cli_e2e.py`. La commande ciblée a refusé le choix de scénario (argparse exit 2; `runner-allowlist-red.log`). J’ai ajouté uniquement ce nom à la liste contrôlée. Ensuite :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_projects_current_pull_request_milestone_from_api
```

Résultat : 1/1 passe (`pr-milestone-process-e2e.log`). Le runner compile offline dans Bubblewrap puis exécute exactement le test ignoré par défaut sous `--unshare-net --unshare-pid`, avec workspace en lecture seule, `/home`/`/tmp` masqués et limites de ressources. Il ne contactait donc que le receiver et le faux endpoint loopback du test.

Autres vérifications : `cargo fmt --all -- --check`, `py_compile` du runner et des scripts de garde de licence, `git diff --check` passent (`static-checks.txt`). `python3 scripts/test_cargo_license_state.py` passe 12/12 (`cargo-license-policy.log`); `python3 scripts/verify_cargo_license_state.py` retourne `metadata_policy_verified` (`cargo-license-state.json`). Cargo 1.95 / lockfile v4 conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed` et `NOASSERTION`. Aucun choix de licence n’est fait. `Cargo.toml` et `Cargo.lock` avaient déjà des modifications locales au début de ce lot; le code de ce lot ne les édite pas.

Outils, statut du checkout et hashes des sources : `environment.txt`, `source-sha256.json`. Les index locaux/projet/worktree valident les 12 fichiers de preuve, préservent leurs préfixes antérieurs et l’index projet lie le SHA courant de l’index worktree (`manifest-check.txt`).

## Portée et limites

C’est un E2E de processus local avec webhook signé par secret de fixture et API simulée; pas une émission native par Forgejo, ni preuve de sémantique upstream au runtime. Forgejo n’était pas dans le `PATH`; aucun AgentOS/LLM n’est utilisé. Le test est `#[ignore]` dans la suite normale et doit passer par le runner isolé. Il prouve uniquement cette lecture courante pour une PR milestone assignée; le chemin `demilestoned`/null n’est pas composé ici. Le résultat ne démontre ni correction du code métier, ni sécurité multi-tenant générale, ni bénéfice comparatif.
