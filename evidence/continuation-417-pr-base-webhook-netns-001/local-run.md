# CONTINUATION-417 — retest du refus base PR dans un namespace isolé

Date : 28 octobre 2026. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; branche `local/WEBHOOK-005-canonical-links`. Aucun fichier d’exécution de la forge n’a changé dans ce lot; il ferme la limite d’isolement explicitement laissée par CONT-416.

## Vérification

Le même test processus, `actual_receiver_binds_pull_request_base_repository_but_allows_fork_head`, s’exécute dans Bubblewrap avec `--unshare-net --unshare-pid --new-session --clearenv`. Le sandbox expose `/usr` en lecture seule, `etc/alternatives`, `/dev` et `/proc`; `/tmp` et `/home` sont des tmpfs. Seuls le binaire de test et le binaire receiver sont transmis en lecture seule par des descripteurs hérités aux chemins `CARGO_BIN_EXE` requis. Aucun répertoire du dépôt, credential, socket Forgejo ou donnée `.runtime` n’est monté.

Résultat observé : **1 passé, 0 échec, 202 filtrés**. Le scénario exerce une PR dont la base correspond au dépôt cible et dont la tête est un fork, puis trois refus d’ingress (base étrangère, base non-objet, `base.repo.full_name` non textuel) suivis de retries corrigés sous les mêmes IDs. Les quatre admissions valides finissent aux séquences 1–4; le test vérifie l’absence de fuite du marqueur privé et l’absence d’append pour chaque refus.

La sortie brute est `isolated-process-final.log`; la commande complète est `commands.txt`. Versions système et hashes des sources/binaries sont `environment.txt`, `source-hashes.sha256` et `binary-hashes.sha256`. La compilation reste hors ligne et dépendances verrouillées; les binaires retestés viennent de `/dev/shm`.

## Portée et limites

Cette preuve établit l’exécution du test receiver dans un namespace réseau/PID local sur cet hôte, avec répertoire temporaire isolé et environnement effacé. Elle n’établit pas une politique de confinement universelle, une résistance à l’évasion/noyau compromis, l’isolation du processus d’un agent exécutant du code hostile, l’émission d’un webhook Forgejo natif, une instance Forgejo live, un LLM/AgentOS ou une fusion. CONT-416 reste la preuve des contrats HTTP et des 147 tests ciblés; CONT-417 ne les rejoue pas.
