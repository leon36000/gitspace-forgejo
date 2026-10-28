# WEBHOOK-JSON-DUPLICATE-MEMBER-001 — refuser les noms JSON ambigus avant append

Date : 25 octobre 2026. Dépôt produit `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; le worktree était fortement modifié. Aucun changement n’a été committé.

## Problème et correction

L’ingestion vérifiait le HMAC des octets puis décodait en `serde_json::Value`. Ce parcours acceptait un objet contenant deux membres `repository.full_name`; le dernier écrasait le premier dans la map. Des parseurs peuvent interpréter autrement les objets à noms dupliqués. RFC 8259 §4 recommande des noms uniques et décrit le comportement des implémentations comme imprévisible lorsqu’il y a des doublons ([source primaire](https://www.rfc-editor.org/rfc/rfc8259.html#section-4)).

`src/webhook.rs` utilise maintenant un visiteur serde récursif pour les tableaux et objets. Chaque nom est décodé en `String` avant comparaison; un nom déjà présent renvoie une erreur de désérialisation mappée à `WebhookError::InvalidJson`. La vérification HMAC intervient avant ce parsing; l’appel ne valide pas le payload ni n’écrit dans `EventJournal` après une erreur. Les types JSON usuels (`null`, booléens, chaînes, nombres, tableaux et objets) sont reconstruits en `serde_json::Value` pour préserver l’interface de validation existante.

## Preuve ciblée et suite

Deux tests d’intégration passent par la fonction publique `ingest_forgejo_delivery`, une signature HMAC valide, et un `EventJournal` réel sur un fichier temporaire. Le premier répète littéralement `full_name`; le second utilise `full_name` puis `full\u005fname`, qui se décodent vers la même clé. Dans les deux cas le résultat est `InvalidJson` et `journal.replay()` reste vide. Résultat ciblé : 2 réussis, 0 échec. Le test `actual_receiver_rejects_signed_duplicate_json_members_before_append` démarre ensuite le vrai binaire `forgejo-webhook-local` en sous-processus, signe deux requêtes synthétiques, observe HTTP 400 `payload_invalid` pour chacune, compare le fichier inbox inchangé octet pour octet et confirme un replay vide : 1/1 réussi.

Après ajout du test de processus, la dernière suite `cargo test --offline --locked --all-targets -- --test-threads=1` passe 488 tests, 0 échec et 55 ignorés sur 34 groupes. `all-targets-process-integration.log` inclut le nouveau cas; `all-targets.log` conserve le passage précédent à 487 tests. Les cas explicitement ignorés ne sont pas exécutés par la commande générique. `cargo fmt --all -- --check` et `git diff --check` passent. Clippy de la bibliothèque passe avec `too_many_arguments` et `collapsible_if` autorisés; les commandes all-targets strictes restent bloquées par des lints préexistants ailleurs dans le dépôt, notamment `manual_div_ceil` dans `src/bin/forgejo-mission-local.rs`, `format_in_format_args` dans `src/mission_012_host.rs` et `type_complexity` dans `tests/webhook_receiver_process_e2e.rs`. Les journaux de ces essais sont conservés dans ce dossier; aucun de ces échecs n’est attribué au parseur du lot.

Le garde de licence rapporte `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`; ses sept tests passent. Aucun choix de licence n’a été fait.

## Rouge TDD et limites de la preuve

Pendant l’implémentation, le test littéral a d’abord échoué sur l’assertion `InvalidJson` avec le parseur précédent. La sortie brute de ce run n’a pas été sauvegardée; ce fait est noté comme observation de développement, pas comme artefact reproductible. Le résultat vert est reproductible avec `commands.txt` et `focused-green.log`.

La preuve combine le contrat d’ingestion et le processus receiver sur port loopback avec fichiers temporaires; elle n’utilise pas Bubblewrap et ne déclenche pas un hook Forgejo vivant. Elle ne vérifie pas exhaustivement les ressources/performances du parseur; la limite de corps d’un mégaoctet reste appliquée avant parsing. Elle ne démontre aucun parcours LLM, aucune intégration AgentOS/Forge Code/RAG/mémoire et aucun gain face à Forgejo + agent + CI conventionnels. Aucun code Forgejo/GitSpace n’est copié ou modifié; aucun service externe, secret, donnée privée, coût nouveau, publication ou environnement de production n’a été utilisé.

## Environnement

Rust `1.95.0`, Cargo `1.95.0`, Python `3.12.3`; les commandes et empreintes de code sont dans `commands.txt`, `tool-versions.txt` et `source-sha256.txt`. Le manifeste `SHA256SUMS` couvre 25 artefacts et passe 23/23. Le worktree Git n’est pas propre; ces résultats ne prétendent pas correspondre à un commit dédié.
