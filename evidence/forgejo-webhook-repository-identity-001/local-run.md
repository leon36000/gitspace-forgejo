# FORGEJO-WEBHOOK-REPOSITORY-IDENTITY-001 — refus d’identité dépôt contradictoire

Date : 25 octobre 2026 (America/Montreal). Worktree fortement modifié, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun staging, commit, merge ou changement de branche.

## Lacune et correction

`src/webhook/reconcile.rs::parse_resource_ref` ne dérivait la portée que de `repository.full_name`. Il ne contrôlait pas les deux identifiants redondants fournis par les webhooks Forgejo v17.0.0 : `repository.name` et `repository.owner.login`. Un corps HMAC valide portant par exemple un `full_name` allowlisté et un nom/owner différent passait la validation puis devenait une entrée durable. À la relecture, il pouvait provoquer une lecture API de la ressource du `full_name` avant qu’un contrôle aval ne remarque une anomalie.

La fonction compare maintenant, si ces champs sont présents, `name` et `owner.login` aux composants de `full_name`. Les divergences deviennent `ResourceMismatch`, avant l’append HMAC et également lors de la résolution d’une entrée déjà journalisée, donc avant le GET de l’adaptateur. Les champs absents et un `owner: null` demeurent tolérés pour la compatibilité; une valeur mal typée est refusée. Le GET Forgejo reste la source de vérité pour l’état de l’issue/PR/branche : cette vérification ne fait qu’écarter une enveloppe contradictoire.

## Références source et interfaces existantes

Forgejo officiel v17.0.0 est épinglé au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les blobs lus via `git show <commit>:<path>` sont `modules/structs/repo.go::Repository` (Owner, Name, FullName), `modules/structs/user.go::User.UserName` (`json:"login"`) et `services/convert/repository.go::ToRepo` (renseigne Name et FullName). SHA-256 des blobs : [source-refs.txt](source-refs.txt). Le scratch contient des POC expérimentaux non liés; aucun fichier de Forgejo n’a été copié ou modifié. Aucun test/build upstream Go n’a été exécuté dans ce lot.

Aucun symbole AgentOS n’a été trouvé dans le manifeste Cargo ou les sources Rust du noyau lors d’une recherche ciblée. Le script `scripts/run_mission_012_live_mcp_agent_e2e.py::build_agent_command` expose deux choix : le fixture `local-test-adapter` et un pont Claude CLI; c’est un driver de test, pas une interface AgentOS. Aucun driver ni modèle n’a été lancé et aucun coût n’a été engagé. Les tests de cette tranche emploient une inbox HMAC temporaire et une API `FixtureApi` déterministe, explicitement une fixture, pas un agent/LLM ni Forgejo live. Détails du contrôle borné : [runtime-interface-check.txt](runtime-interface-check.txt).

## TDD et vérifications

Les tests ont été écrits avant l’implémentation :

- `signed_repository_identity_conflicts_are_rejected_without_journal_side_effect` échouait : l’ingestion appendait la livraison contradictoire au lieu de renvoyer `InvalidEventPayload`; aucune assertion de compilation n’a échoué. [Journal rouge](red-ingestion.log), code `101`.
- `repository_identity_conflicts_are_rejected_before_api_call` échouait : la réconciliation atteignait `FixtureApi`, puis échouait sur le champ absent `number` (`InvalidCurrentStateField`) au lieu de refuser plus tôt. [Journal rouge](red-reconcile.log), code `101`.
- Après correction, les deux contrats ciblés passent : [9 tests d’ingestion webhook](focused-green.log), dont l’acceptation de champs concordants, et **35 tests de réconciliation**, dont les refus des deux divergences avant lecture API. Code `0`.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : **529 réussis, 0 échec, 65 ignorés**, sur 34 groupes. Les tests ignorés (dont des tests processus nécessitant leurs runners spécifiques) ne sont pas couverts par ce résultat. [Sortie complète et résumé](all-targets.log).
- `rustfmt --edition 2024 --check` sur les trois fichiers Rust et `git diff --check` passent.
- `cargo clippy --offline --locked --all-targets -- -D warnings` retourne `101` à cause de **12 lints du worktree préexistants hors du helper ajouté** : 8 `too_many_arguments`, 2 `collapsible_if`, 1 `if_same_then_else` dans l’ancienne table d’événements, et 1 `bool_assert_comparison`. Ils n’ont pas été modifiés dans ce lot. Clippy sur le lib et les deux contrats passe en autorisant seulement ces quatre catégories; voir `clippy-focused-existing-lints-allowed.log`.
- Le garde Cargo hors ligne passe : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`. Readback : [license-state.json](license-state.json) et [metadata-readback.json](metadata-readback.json). Aucune licence choisie.

Tous les tests et Clippy ont tourné avec Bubblewrap 0.9.0, namespace réseau séparé, environnement effacé, racine en lecture seule, `target/` seul montage inscriptible, Cargo hors ligne et lockfile verrouillé. Commandes et versions : [commands.txt](commands.txt), [toolchain.txt](toolchain.txt). Les sorties de code et les empreintes finales sont conservées dans ce dossier.

## Manifestes append-only

Le manifeste local du dossier vérifie 28 fichiers. `verify_indexes.py` conserve le dernier hash valide par chemin et valide les index complets : worktree 5 346 chemins uniques, 7 lignes historiques non hashables préservées; parent 836 chemins uniques, 2 lignes historiques non hashables préservées; zéro chemin absent ou dernière empreinte divergente. Les lignes courantes ont été ajoutées en fin de fichiers; aucun historique n’a été réécrit.

## Portée et suite

Ce résultat démontre le refus d’une identité de dépôt interne contradictoire dans les contrats d’ingestion/relecture et évite l’appel API fixture dans ce cas. Il ne teste ni une livraison HTTP réelle provenant d’une instance Forgejo, ni un run de l’adaptateur contre Forgejo v17, ni l’authenticité de l’émetteur si le secret HMAC est compromis, ni un agent réel. Le comportement post-merge, AgentOS/LLM et le benchmark conventionnel ne sont pas concernés et restent ouverts. Aucun secret n’a été envoyé à un service externe; aucun service externe, runner CI, instance persistante ou dépôt de production n’a été modifié. Projet incomplet.
