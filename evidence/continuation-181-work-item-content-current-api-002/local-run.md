# CONTINUATION-181 — contenu issue/PR tiré de l’API courante

Date du run : 2026-10-27 (heure locale Montréal; détails d’environnement dans `environment.txt`). Le worktree était déjà fortement modifié; HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, déploiement ou changement Forgejo upstream n’a été fait.

## Résultat

Le contrat de réconciliation projette désormais un contexte optionnel `canonical_state.work_item_content` pour issue, PR et commentaires avec parent issue/PR. `title` et `body` sont obtenus uniquement par GET courant; le webhook ne fait pas autorité. La projection porte l’origine de route et `trust=untrusted_user_content`, limite le titre à 1 024 octets et le corps à 65 536 octets, distingue body explicitement null de propriété absente, omet un contenu trop long sans effacer le reste de l’état, et refuse les types invalides. Le contexte reste distinct des exigences versionnées protégées. Single/batch sont versionnés 8/9, capacités v3.

Le run live sur l’instance jetable Forgejo a reçu l’événement réel `issues/opened`; l’enveloppe persistée contient le titre/corps initiaux. Après fermeture et édition de cette issue, la livraison `opened` ancienne a été réconciliée. Le JSON final donne l’état courant `closed` et les valeurs API éditées, avec `source=current_forgejo_issue_api` et `trust=untrusted_user_content`, différentes de celles du payload initial. Le readback PR courant expose aussi son titre/corps (`current_forgejo_pull_request_api`). Le parent d’un commentaire PR est vérifié par readback courant. Les comparaisons sont dans `work-item-content-readback.json`, `edited-closed-event-reconciliations.json` et `pull-request-open-event-reconciliation.json`.

Le runner a terminé avec code 0 et sept opérations/tâches enregistrées dans `result.json`. Il a arrêté le receiver et Forgejo, désactivé le webhook et vérifié l’absence des listeners et d’erreur de nettoyage dans `port-cleanup-readback.json`. Forgejo était le binaire `/tmp/forgejo-17.0.0`, version affichée `17.0.0+gitea-1.22.0`, Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit exact de build inconnu. L’audit antérieur de la source Forgejo v17.0.0 est ancré au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`; cette observation n’a pas patché Forgejo.

## Vérifications

- Le log `cargo-all-targets.log` du run complet comporte 36 groupes : 651 réussis, 0 échoué, 121 ignorés. Ce passage précède les deux dernières assertions ajoutées au contrat; après ces assertions, le contrat est relancé séparément et passe 74/74 (`rust-content-contract-final.log`). Aucun code de production n’a changé entre ces deux exécutions.
- Réconciliation CLI : 151/151 (`rust-reconcile-cli.log`). Capacités E2E : 3 réussis, 0 échec, 2 ignorés parce qu’ils réservent un port fixe (`rust-capabilities-e2e.log`).
- Python emitted webhook + replay natif : 158/158 (`python-emitted-replay.log`). `py_compile`, `cargo fmt --all -- --check`, `git diff --check` et la garde Cargo terminent avec code 0; la garde de licence passe 9 tests (`pycompile.log`, `rustfmt.log`, `diff-check.log`, `cargo-license-state.txt`, `cargo-license-guard-tests.log`). Metadata et état Cargo : `cargo-metadata.json`; `license=null`, `license_file=null`, `publish=[]`, choix de licence `no_license_selected`, droits de redistribution `not_assessed`.

## Portée et limites

C’est une observation locale `n=1`, même hôte, instance jetable et tâche fixture. Le receiver/CLI/Forgejo sont exécutés localement; aucun code candidat hostile n’a été lancé. Le GET courant et les autres lectures ne forment pas un snapshot transactionnel; une modification ultérieure peut déjà rendre ce contexte périmé. Marquer la donnée comme non fiable ne neutralise pas une injection de prompt. Cette tranche ne démontre ni agent LLM/AgentOS/RAG/mémoire, ni merge, ni comparaison de productivité ou de coût, ni gain général. La propriété testée est une projection de contexte, pas une exigence.

Les versions et commandes sont enregistrées dans `environment.txt` et `command.txt`; les empreintes initiales et finales sont dans `source-hashes.txt` et `final-source-hashes.txt`. `provenance.json` lie le binaire exécuté et l’état du dépôt observé. La revue du flux de données après la sérialisation est dans `consumer-audit.md` avec ses propres empreintes et résultats. Le reçu local et les index append-only sont vérifiés par les scripts fournis dans ce dossier.
