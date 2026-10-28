# CONTINUATION-130 — refus batch avant sélection sur inbox tronquée

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. L’arbre était sale et plusieurs sources de test non suivies; aucun staging, commit, reset ou nettoyage n’a eu lieu. Le hash précis de la source de test est dans `source-hashes.txt`.

## Lacune et scénario

CONTINUATION-128 avait un refus-processus single sur une queue tronquée avant validation de quarantaine. Le nouveau test `real_cli_preserves_an_incomplete_tail_for_an_unresolved_batch_selection` couvre l’enveloppe batch : une inbox synthétique complète a un ID d’issue valide puis une queue finale interrompue; le CLI reçoit l’ID présent et un second ID absent, avec un FD de credential synthétique.

Le vrai binaire retourne code 1 et schema 6 `inconclusive/inbox_journal_invalid`, compte les deux IDs, fait zéro lecture amont, et déclare aucun état canonique, lecture de credential, appel amont ou réparation. Le test compare l’inbox byte-for-byte et relit tous les octets du pipe sans consommation. Il n’y a pas de serveur Forgejo ni d’écoute réseau dans le scénario. Si le vieux `replay()` était réintroduit, il réparerait le suffixe puis échouerait sur l’ID absent; la résolution complète du batch précède le credential et toute construction du client, ce qui évite une requête réseau même dans ce cas.

Le target `cargo test --offline --locked --test webhook_quarantine_cli` passe 4/4 : ce test, le refus single sur disposition invalide + queue tronquée, le refus des dispositions malformées/détachées, et la quarantaine d’une ligne invalide. Le test-only addition n’a pas modifié la logique runtime; la suite complète `--all-targets` n’a pas été répétée. `cargo fmt --all -- --check` passe après `cargo fmt --all`; `git diff --check` passe.

## Métadonnées de licence

Readback frais `cargo metadata --offline --locked --no-deps --format-version 1` et `python3 scripts/verify_cargo_license_state.py` confirment pour `agent-forge-core` version 0.1.0 : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`, export `NOASSERTION`. Les neuf contrats de `python3 -m unittest -v scripts.test_cargo_license_state` passent. Aucune licence produit n’est choisie et aucune publication n’est rendue possible par Cargo. Cette vérification ne détermine pas les droits GitSpace/Forgejo ou dépendances.

## Portée et limites

C’est un test processus local du CLI et une inbox construite avec des fixtures, pas une réconciliation de deux ressources existantes sur Forgejo. Il protège le parsing complet et la forme batch avant sélection; il ne teste pas le traitement positif d’un batch multiressource ni un writer externe qui contourne le verrou. Les résultats `--all-targets` précédents sont 622 réussis, 0 échec, 106 ignorés sur 36 groupes avant cette nouvelle assertion; ce rapport n’ajoute pas un nouveau total global. Pas de modèle, AgentOS, réseau externe, secret réel, coût, production ou changement de licence.

Le checkpoint et la matrice référencent cette preuve. Le prochain lot doit choisir une autre lacune non couverte sans répéter les scénarios replay, egress et merge déjà établis.
