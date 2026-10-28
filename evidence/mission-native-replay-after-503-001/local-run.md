# CONTINUATION-195 — admission de mission après un 503 et replay natif

Date : 27 octobre 2026. Dépôt produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Le lot ajoute un mode opt-in au runner Python et des tests de contrat; il ne modifie ni le noyau Forgejo, ni le code Rust, ni les métadonnées de licence.

## Lacune ciblée

CONTINUATION-194 démontrait un `503` réel puis deux replays manuels et la réconciliation des UUID frais vers une seule issue. CONTINUATION-193 vérifiait localement le même refus au niveau receiver. Il restait à composer la reprise avec le ledger de mission : admettre le premier replay acceptable, puis prouver qu’un nouvel UUID issu du second replay ne crée pas une seconde allocation.

## Changements et parcours

Le nouveau mode `--receiver-overload-first-delivery` exige `--native-replay-admission`. Le runner démarre le receiver Rust, un proxy loopback de mesure et Forgejo local. Il remplit les quatre workers et les seize emplacements de file, crée une issue puis lit la tâche Forgejo en lecture seule. La première requête reçoit réellement `503` et `Retry-After: 5`; aucun record n’est écrit. Après le drain (`4×408`, `16×405`), une fenêtre de 6 s n’observe ni task ni POST supplémentaire.

Une session Web locale authentifiée rejoue la tâche en échec. Le replay garde le même contenu (SHA-256 brut et sémantique) et produit une UUID distincte; son unique delivery est acceptée en `202`, devient la séquence 1 du journal, puis est admise par le CLI avec l’exigence `MISSION-012-CLAMP-001` déjà versionnée. Le replay natif de cette delivery acceptée émet une troisième UUID et un second `202`. Le CLI renvoie `already_claimed`; les empreintes de l’admission et du journal d’exécution restent identiques. Le reçu final liste la tâche 503 échouée, la tâche replay admise, la dernière task replay et les deux records réellement acceptés.

## Échecs conservés et correction

- Les tests rouge initiaux montrent l’absence de la fonction de liaison et du drapeau de CLI avant implémentation (`tdd-red.log`, `tdd-red-parser.log`).
- `live-run-001` a produit le vrai `503`, sans record journalisé, puis a arrêté Forgejo, le proxy et le receiver sans listener résiduel. Le run s’est arrêté parce que le harnais comparait la liste issue-only et une autre projection SQLite enrichie de hashes. La lecture différée confirme un seul task en échec et un journal toujours vide; ce n’était pas une correction applicative requise.
- Le harnais compare désormais la même projection pour détecter les retries, utilise le readback enrichi seulement pour lier les hashes du payload, et passe à son validateur la liste de journal réellement relue avant le replay. La validation exige aussi que les deux demandes observées soient des `POST /forgejo/webhook` de type `issues`. `live-run-002` avait réussi avant ce durcissement; le run final `live-run-003` passe avec le parcours complet et ces bindings supplémentaires.

Les chemins d’échec et de succès sont gardés sous `live-run-001/`, `live-run-002/` et `live-run-003/`; les trois états jetables `/tmp/forgejo-mission-native-replay-after-503-001`, `-002` et `-003` ont été conservés, non nettoyés.

## Vérifications reproductibles

- `python3 -m py_compile scripts/run_forgejo_live_webhook_mission_admission.py tests/test_forgejo_live_webhook_mission_admission.py` — passe.
- `python3 -m unittest discover -s tests -p 'test_forgejo_live_webhook_mission_admission.py'` — 32 tests, 0 échec.
- `python3 -m unittest discover -s tests -p 'test_forgejo_webhook_native_replay_e2e.py'` — 30 tests, 0 échec.
- `cargo metadata --offline --locked --all-features --no-deps --format-version 1` et `python3 scripts/verify_cargo_license_state.py` — passent. `agent-forge-core` garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, marqueur `NOASSERTION`.
- `git diff --check` sur les deux scripts/tests et le checkpoint — passe; la matrice non suivie est contrôlée sans trailing whitespace/NUL dans le log de vérification.
- Chaque reçu du run final passe `sha256sum -c live-run-003/SHA256SUMS`; les manifestes globaux append-only et leur vérificateur sont dans ce dossier.
- Commande E2E exacte et code de sortie 0 : `runner-command-003.txt`, `runner-exit-code-003.txt`, `runner-output-003.log`.
- Le runner a compilé les exécutables Rust de receiver et d’admission dans son build Bubblewrap; l’empreinte du log de build et les hashes des binaires sont dans `live-run-003/provenance.json`. Python 3.12.3, Rust/Cargo 1.95.0 et l’outil Bubblewrap sont consignés sous `verification/runtime-versions.txt`.

L’E2E rapporte Forgejo `17.0.0+gitea-1.22.0`, Go `1.26.9`, binaire `/tmp/forgejo-17.0.0` SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit de construction du binaire est inconnu. Le run lie ses exécutables et ses sources à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. La source Forgejo n’est pas réexaminée ici; l’audit antérieur v17.0.0 est référencé dans CONTINUATION-194.

## Portée et limites

Parcours sur instance locale jetable `n=1`, sans agent réel, modèle, AgentOS, RAG ou mémoire. Ce runner s’arrête à l’admission : il n’exécute pas de candidat, ne crée pas de PR, ne vérifie pas indépendamment un changement et ne fusionne pas. Le readback des tâches utilise la table SQLite locale de Forgejo, non une API publique stable. Forgejo, receiver, proxy et CLI sont sur le même hôte/UID; la vérification des listeners confirme uniquement la fermeture de ce scénario. La fenêtre de 6 s ne prouve pas l’absence de retries différés. Aucun gain, réduction de tokens, coût, effet général ou sécurité de production n’est établi. Pas de nouvel appel payant, de publication ou de changement de production; aucun code upstream copié/adopté, aucun choix de licence fait. Le projet reste incomplet.
