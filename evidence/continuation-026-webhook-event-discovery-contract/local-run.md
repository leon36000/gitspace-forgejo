# CONTINUATION-026 — découverte machine-readable des événements webhook

Date : 26 octobre 2026. Produit `agent-forge-core`, worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de départ `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Lacune et changement

Avant ce lot, `GET /forgejo/capabilities` annonçait les neuf noms d’événements, mais les formes nécessaires et contraintes restaient dans du texte d’erreur. Le receiver sérialise maintenant `SUPPORTED_EVENT_CONTRACTS` dans `webhook_ingest.event_contracts` avec schéma v1. Chaque entrée indique l’événement, le type de ressource, les champs signés requis, les contraintes de payload, le chemin API canonique et l’interprétation inconclusive d’un 404.

Le contrat rend explicites les détails utiles au client : `push` exige `refs/heads/<branch>` et des identifiants `before/after`, mais le readback Forgejo fait autorité; `create`/`delete` acceptent uniquement `ref_type=branch` et un nom court; les événements d’approbation, de rejet et de commentaire exigent le `review.type` correspondant. Le 404 reste inconclusif parce qu’un dépôt privé peut masquer l’accès. Le nouveau champ est additif au document de capacités v1. La validation des entrées, les lectures de réconciliation et le journal ne changent pas.

## Vérification

TDD : le test contrat ajouté échoue d’abord car `event_contracts` n’existe pas (`test-red.log`, sortie 101). Après ajout du contrat statique et de sa publication HTTP :

- `capabilities_publish_machine_readable_contracts_for_every_supported_event` passe 1/1. Il compare les neuf noms et leur ordre aux `SUPPORTED_EVENTS`, vérifie pour chacun le type de ressource, les champs requis, la route de lecture et le traitement du 404, puis contrôle des contraintes clés de `push`, `delete` et des revues.
- `capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox` passe 1/1 au vrai processus `forgejo-webhook-local`.
- `cargo test --locked --offline --all-targets -- --test-threads=1` passe 553 tests, 0 échec, 76 ignorés, 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent.

Les sorties intégrales sont conservées dans ce dossier. Les contrôles s’exécutent sur le worktree local fortement modifié; aucun commit ni publication n’est effectué.

## Cargo et limites

Readback `cargo metadata --locked --offline --no-deps`: paquet `agent-forge-core` version `0.1.0`, `license=null`, `license_file=null`, `publish=[]`. Le garde renvoie `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`; aucune licence n’est choisie.

La preuve est statique et locale : aucun serveur Forgejo, modèle, AgentOS, service externe, coût, credential utilisateur, modification de production ou fusion n’intervient. Le processus receiver prouve le JSON local et l’absence d’append, pas que toutes les versions Forgejo émettent ces formes ni qu’un agent réel sache les utiliser. Les 76 tests ignorés n’ont pas été exécutés par `--all-targets`. Le parcours LLM et la comparaison contrôlée restent non démontrés; le projet demeure incomplet.
