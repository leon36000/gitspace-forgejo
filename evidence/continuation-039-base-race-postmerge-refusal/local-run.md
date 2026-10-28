# CONTINUATION-039 — course de base pendant le POST de fusion

Date : 26 octobre 2026. Checkout isolé `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; nombreux changements antérieurs conservés. Aucun commit ou déploiement.

## Observation

Un test processus déterministe place l’avance externe de `main` après la dernière lecture SHA côté client et pendant l’arrivée du POST de fusion. La phase rouge, qui exigeait qu’aucune mutation ne survienne, échoue sur `fixture.merged()`: l’endpoint synthétique applique le POST sur le nouveau parent `4444444444444444444444444444444444444444`, différent de la base validée `1111111111111111111111111111111111111111`.

Le test final caractérise exactement cette limite. Le vrai processus `forgejo-reconcile-local` ne publie pas `merged`: il retourne `inconclusive / merge_outcome_not_confirmed`. Une reprise lit seulement l’état existant, ne réémet pas de POST et reste inconclusive; le journal conserve un intent et aucune résolution terminale. Le test ciblé passe sous Bubblewrap, réseau et PID isolés, avec source et binaire de test montés en lecture seule. Le test précédent de reprise après réponse de merge perdue repasse également après la modification du fixture.

## Interprétation et limite

C’est une preuve négative, pas une validation de sécurité : le résultat fail-closed empêche une fausse déclaration de succès après coup, mais ne prévient ni n’annule la mutation sur une base non validée. Le test HTTP est une fixture déterministe, pas Forgejo. L’audit source séparé de Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571` conclut que le CAS expérimental ne couvre pas toutes les routes d’écriture et que l’API publique de merge n’impose pas cette précondition; voir [l’audit des writers et LFS](../forgejo-v17-writer-lfs-audit-001/source-audit.md). Le scratch reste inchangé par ce lot. Go est absent du `PATH`, donc aucun test Go ni test Forgejo live n’a été lancé.

Le correctif sûr exige une précondition atomique appliquée par le serveur à tous les writers pertinents, et un protocole LFS récupérable. Une relecture client supplémentaire laisserait une fenêtre de course. Aucun changement de code de production n’a été fait ici; la capacité de fusion contrôlée n’est donc pas démontrée contre un writer Forgejo non coopératif et le projet reste incomplet.

## Vérifications et provenance

- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only external_base_advance_during_merge_post_is_fail_closed_but_still_mutates_branch --only killed_cli_after_forgejo_merge_recovers_readback_only_and_serializes_duplicate_retries` : 2 tests processus réussis, chacun 1/1; compilation hors ligne/verrouillée depuis l’arbre source staged, exécution sous `/usr/bin/bwrap --unshare-net --unshare-pid --new-session`.
- Test rouge précédent : même runner avec uniquement le test de course, attente d’absence de mutation; échec attendu parce que le fixture a accepté le POST et déplacé la base du commit parent.
- `rustfmt --edition 2024 --check tests/forgejo_merge_post_crash_recovery_process_e2e.rs`, `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py` et `git diff --check` : vérifications locales ciblées.
- Rust `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, Linux ARM64.
- Les logs red/green et deux premiers logs de mise au point sont dans ce dossier. Ils ne contiennent que le fixture, les entrées synthétiques et les commandes isolées. L’empreinte source staged du run final est imprimée dans `focused-isolated-run.log`; `source-sha256.txt` lie les fichiers locaux restants.

La fixture et la reprise testée ne prouvent pas le comportement de toutes les routes Forgejo. Aucun modèle, AgentOS, secret réel, service externe, coût, production ou publication n’a été engagé. Pas de gain comparatif mesuré.
