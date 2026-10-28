# CONTINUATION-285 — frontière agent et webhook signé sur le source courant

Date : 27 octobre 2026. Travail local uniquement dans le worktree existant, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié. Aucun commit, push, accès production, appel modèle, dépense ou service externe.

## Décision et cause

Le harnais du parcours PR live n’ajoutait pas de signature Ed25519 aux livraisons du receiver. La version courante de `forgejo-reconcile-local` refuse maintenant, avant lecture du credential et tout appel API, une livraison sélectionnée sans politique de clé de confiance. Le premier run échouait avec un message générique; après ajout d’un reçu diagnostic limité à des champs non sensibles, le run rouge a identifié `host_journal_attestation_policy_missing`, `upstream_read_attempts=0`. Voir `forgejo-composed-run.log`, `forgejo-composed-diagnostic-run.log` et `forgejo-composed-error-run.log`.

Le harnais crée maintenant une graine de 32 octets dans un fichier host mode `0600`, dérive la clé publique par `forgejo-webhook-local --print-attestation-public-key`, configure le receiver pour signer les lignes et transmet uniquement la clé publique au reconciler. Le receiver redémarré reçoit la même graine. La graine reste dans l’état host sous `/tmp`; la commande Bubblewrap du test monte un tmpfs neuf sur `/tmp` et ne monte pas l’état host. Un reçu donne l’algorithme et l’empreinte de la clé publique, jamais la graine ni la clé privée. Il n’y a pas de probe distinct qui tente d’ouvrir ce fichier depuis le processus agent.

Le test Python révélait aussi une attente interne au harnais devenue périmée : le résultat du probe ajoute `host_systemd_bus_visible=false`, mais l’assertion comparait encore tout le dictionnaire à l’ancien schéma. La sonde confirmait déjà les namespaces distincts, la liste de routes bloquées, les descripteurs `[0,1,2]` et un processus vivant. L’attente a été synchronisée avec le schéma courant; les 17 tests Python passent ensuite.

## Preuve Forgejo composée

Le binaire local `/tmp/forgejo-17.0.0` annonce `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; il est construit avec Go 1.26.9 selon `--version`. Son commit de build n’est pas établi. L’audit précédent du source officiel v17.0.0 est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; ce hash de source n’établit pas la provenance binaire.

Deux instances Forgejo temporaires séparées ont été démarrées sous `/tmp` avec les modes `--non-admin-merger --stdio-mcp-egress-during-merge --observe-pr-webhook` :

1. `forgejo-composed-fixed-run.log` : PR locale #2 fusionnée par le rôle non-administrateur autorisé. Le contrôle métier initial échoue de façon actionnable (`baseline_exit_code=101`); après correction, les **9 tests d’acceptation passent**. Le readback Forgejo confirme issue fermée, PR fermée/merged, la base et `main` au même commit de fusion. La livraison native `pull_request/closed` obtient HTTP 202 à la séquence 4. Le reconciler vérifie l’attestation, fait exactement un GET, renvoie `current_state_read` et lie ID de livraison, dépôt, PR, tête, base et commit fusionné. Les 13 contrôles individualisés du reçu `webhook-reconciliation-diagnostic.json` sont vrais.
2. `forgejo-composed-restart-replay-run.log` : après arrêt propre du receiver, le nouveau processus récupère une fois la livraison de la même inbox, garde le même listener et la même graine. Le readback avant/après garde quatre enregistrements et le même hash de journal. Le rejeu re-signé de test reçoit HTTP 202, conserve ID/séquence 4, est classé duplicate et ne modifie pas l’inbox. L’attestation du reçu sélectionné est vérifiée et la réconciliation canonique réussit ensuite.

Dans les deux parcours, le même processus `stdio-MCP` fixture reste actif pendant le gate. Le résultat egress indique `network_connected=false`, routes API/Web/`git-receive-pack` bloquées, credential de fusion non visible, zéro connexion de route observée par le superviseur, et aucun FD au-delà de 0–2. Le token n’est ni sérialisé en configuration/commande/environnement, ni hérité par le processus stdio, ni écrit en fichier; le probe de credential échoue depuis le namespace agent. Le gate host appelle sa route de merge, reçoit HTTP 200, et le readback post-fusion exécute les mêmes octets source. Le receiver se termine avec code 0.

## Vérifications

- `cargo test --locked --offline --all-targets` : **37 suites, 693 réussis, 0 échec, 151 ignorés**; log complet `cargo-all-targets.log`.
- `python3 -m unittest discover -s tests -p 'test_forgejo_live_pr_merge_e2e.py' -v` : **17 réussis** après la correction du dictionnaire de vérification; voir `runner-unit-tests-final.log`. Le premier run rouge (1 erreur sur l’attente de schéma) est conservé dans `runner-unit-tests.log` et le ciblage dans `sandbox-probe-targeted-rerun.log`.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_forgejo_live_pr_merge_e2e.py` : code 0.
- Rust/Cargo `1.95.0`, Python `3.12.3`, Bubblewrap `0.9.0`, Linux/aarch64.

Les sorties de diagnostic ne sont pas interprétées comme une preuve d’identité du binaire ni de sécurité exhaustive. Le résumé Forgejo marque `external_writers_excluded=false`; l’essai ne démontre pas de fence côté serveur contre tous les writers. Les assertions réseau couvrent les trois routes listées et ce processus/système précis.

## Portée et limites

Le producteur MCP est le `local-test-adapter` déterministe; `llm_invoked=false`. Ce run prouve les interfaces locales sur une tâche fixture, un échec corrigé, un gate indépendant, une PR, un événement réellement émis par Forgejo local, une décision de réconciliation et un merge contrôlé. Il ne prouve pas un parcours avec LLM/AgentOS, un gain comparatif, ni une supériorité générale. Aucune comparaison Forgejo conventionnel appariée, aucun effectif répété ou coût tokens n’est disponible. L’isolation ne couvre pas un noyau/host privilégié compromis, toutes routes, l’identité multi-tenant ou d’autres systèmes d’exploitation. La mission reste incomplète.

Les journaux d’erreur et de réussite sont gardés dans ce dossier. Les états de test sous `/tmp` sont conservés, ne sont pas inclus dans les manifestes d’évidence et contiennent des credentials et la graine privée synthétiques; aucun de leurs octets secrets n’a été lu par l’examen.
