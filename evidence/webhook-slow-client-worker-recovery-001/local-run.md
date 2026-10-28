# CONTINUATION-192 — reprise du receiver après saturation par requêtes incomplètes

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre fortement modifié.

## Lacune examinée

Le receiver possède quatre workers, une file bornée à seize connexions et une échéance totale de lecture de trois secondes. Les tests existants couvraient le délai de `read_request` et la redélivraison complète, mais pas le processus `serve` quand toutes les voies de traitement sont occupées par des requêtes incomplètes. Aucun changement du chemin d’exécution du receiver n’était nécessaire; ce lot ajoute une preuve processus de cette interaction.

## Parcours et résultat

`actual_receiver_recovers_after_all_request_workers_timeout` démarre le vrai binaire `forgejo-webhook-local`, puis envoie quatre connexions contenant une requête HTTP avec `Content-Length: 128` mais seulement un préfixe de corps. Les clients restent ouverts. Après 250 ms pour laisser le listener distribuer les requêtes aux quatre workers, le test soumet une livraison HMAC valide distincte et mesure son attente.

La livraison valide reçoit HTTP 202, `accepted=true`, `duplicate=false`, `sequence=1`, après une attente supérieure à deux secondes et inférieure à six secondes. Chacun des quatre clients incomplets reçoit HTTP 408, `request_timeout`, `retryable=true`. Après arrêt gracieux, la relecture de l’inbox contient une seule entrée, celle de la livraison valide; aucun préfixe incomplet n’a été ajouté. Le test processus a passé trois répétitions, respectivement en environ 3,11 s, 3,06 s et 3,07 s.

## Vérifications

- `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_recovers_after_all_request_workers_timeout -- --nocapture` — 1/1; répété deux fois de plus, 1/1 chaque fois.
- `cargo test --offline --locked --all-targets` — 660 réussis, 0 échec, 124 ignorés, 36 cibles. Le nouveau test n’est pas ignoré et figure dans cette suite.
- `cargo fmt --all -- --check` et `git diff --check` — passent.
- `cargo metadata --offline --locked --no-deps --format-version 1` et `scripts/verify_cargo_license_state.py` — `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX déclaré `NOASSERTION`.

Les logs des trois répétitions sont `focused.log`, `focused-repeat-2.log` et `focused-repeat-3.log`; la suite complète est `all-targets.log`. `toolchain.txt`, métadonnées Cargo, contrôle de licence, empreintes et index append-only sont dans ce dossier.

## Portée et limites

Le test couple réellement le pool de workers, la file d’acceptation, l’échéance de lecture, les réponses HTTP et l’écriture append-only du receiver local. Il s’appuie sur quatre workers, conformément à la constante actuelle `WORKERS=4`; le test doit être ajusté si cette configuration change. Trois runs locaux n’établissent pas un SLO, un essai sous charge soutenue, la résistance à des milliers de connexions ou un comportement réseau distant. La livraison valide est journalisée mais n’est pas ensuite réconciliée avec une fixture Forgejo dans ce test. Aucun serveur Forgejo live, agent LLM/AgentOS, service externe, secret réel ou production n’est impliqué. Aucune licence produit n’est choisie.
