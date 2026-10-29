# CONTINUATION-461 — synchronisation incrémentale vers GitLab PC4

Date : 29 octobre 2026 (America/Montreal). Worktree source : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt source est resté non commité localement; aucune branche locale ni aucun historique n’a été réécrit.

## Accès et cible

Le connecteur `mcp__GitLab_PC4` version `19.5.1` lit le projet privé `root/gitspace-forgejo` (ID 4). Sa branche par défaut demeure `codex/pc6-transfer-2026-10-28`, protégée et non inscriptible pour ce connecteur. Le travail a été envoyé uniquement à la branche non par défaut `codex/pc6-transfer-work-2026-10-28`, non protégée et rapportée inscriptible. La tête lue avant le lot était `9ac877767e9b458a9cd911e67a10f87aeeaefbdb`.

Le connecteur GitLab fourni dans l’environnement a permis les lectures et commits nécessaires. Les courriels et les valeurs de credentials qu’ils contiennent n’ont pas été relus ni réutilisés. Les reçus précédents signalent que les accès trouvés par courriel ne sont pas limités à ce dépôt et que certaines valeurs ont été exposées; ils ne servent pas à ce transfert.

## Audit local et sélection

Le scanner de transfert `evidence/continuation-425-gitlab-connector-preflight-001/audit_transfer_candidates.py` a été relancé depuis ce worktree. Il a inspecté 16 084 chemins indexés ou non suivis non ignorés, soit 15 943 fichiers réguliers et 204 991 787 octets, ainsi que 2 712 membres ZIP. Il n’a trouvé aucune correspondance aux motifs ciblés et aucune archive illisible. Cette recherche est heuristique et ne prouve pas l’absence de tous les formats de secret.

Le différentiel incrémental a examiné 783 chemins modifiés ou ajoutés depuis le dernier transfert connu : 696 fichiers texte exportables étaient comparables, 108 avaient déjà le bon blob sur GitLab et 588 nécessitaient création ou mise à jour. Ces 588 fichiers représentaient 12 546 569 octets. Avant chaque appel d’écriture, le contenu local a été relu, son SHA d’objet Git et son mode vérifiés, puis envoyé avec `encoding=text`. Les contenus Base64 binaires n’ont pas été envoyés.

## Écriture et vérification distante

Onze commits ont été ajoutés à la branche de travail : mise à jour architecture; lots 01–08; checkpoint; manifeste append-only. Après chaque écriture, la tête de branche a été relue et correspondait au SHA du commit retourné. Les messages portent `[skip ci]`; aucun statut CI n’est revendiqué.

La lecture récursive finale de l’arbre GitLab a parcouru 168 pages, 16 799 entrées et 15 733 blobs. Les 588 chemins du différentiel ont tous été retrouvés avec le blob et le mode attendus : 588/588 identiques, zéro absent et zéro divergent. La tête vérifiée avant l’ajout du présent reçu est `58f0ea3550887a8dc125a857f2815afc9834ec0a`.

L’ajout des reçus aux deux index a d’abord mis en évidence trois anciennes empreintes périmées pour les sources CONT-460 (`tests/webhook_receiver_process_e2e.rs`, `scripts/run_webhook_reconcile_cli_e2e.py`, `tests/webhook_reconcile_contract.rs`). Le helper a ajouté leurs nouvelles empreintes en suffixe, sans réécrire le préfixe; la nouvelle vérification append-only passe ensuite sur le worktree et le projet. Résultats : 14 199 chemins worktree et 9 368 chemins projet vérifiés, sans absence, divergence, chemin non sûr ni entrée non hachée; le worktree conserve une ligne historique sans hash et six lignes vides ignorées.

Les commits de transfert sont détaillés dans `remote-commits.txt`; les comptes expurgés sont dans `transfer-result.json`.

## Limites et suite

Ce transfert est une synchronisation d’instantané de fichiers par API, pas une réplication du graphe Git local : les refs, tags et ancêtres PC6 ne sont pas importés. Les 28 archives ZIP (107 151 276 octets dans l’audit antérieur), le bundle du dépôt imbriqué, les caches locaux, `.claude` et `.runtime` restent exclus. Les opérations de transfert précédentes ont montré que l’API MCP écrivait littéralement certaines chaînes Base64; aucune fidélité binaire n’est revendiquée. Le port HTTPS de PC4 n’est toujours pas démontré joignable depuis le shell local, et aucun test ou build n’a été exécuté sur PC4. Le code et les vérifications continuent donc d’être exécutés dans le worktree PC6, puis synchronisés sur cette branche dédiée.

Aucune MR, fusion, modification de branche par défaut ni opération de production n’a eu lieu. Le parcours LLM/AgentOS, l’intégralité binaire, les vérifications CI PC4 et le benchmark comparatif restent à démontrer. Prochaine étape : vérifier les blobs du présent reçu après son append, conserver cette branche comme cible de synchronisation, puis poursuivre les lots locaux sans prétendre que l’exécution a migré vers PC4.
