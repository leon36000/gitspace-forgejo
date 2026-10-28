# GITLAB-PC4-TRANSFER-004 — transfert vérifié jusqu’à CONTINUATION-436 — 28 octobre 2026

## Cible et résultat final

- Projet privé : **root/gitspace-forgejo**, ID 4.
- Branche modifiée : **codex/pc6-transfer-work-2026-10-28**. La branche par défaut protégée **codex/pc6-transfer-2026-10-28** n’a pas été modifiée; aucun merge request ni merge.
- Le snapshot PC6 comprend le checkpoint, quatre fichiers source, le manifeste global append-only, les reçus textuels CONTINUATION-429/435/436 et leurs traces. 80 fichiers, 5 205 980 octets. Les archives binaires, .runtime, .claude, caches, secrets et configuration locale sont exclus.
- Au commit **984a938551759226a4275666adb4eed35e3ddc26**, le readback du tree a comparé les Git blob IDs et modes des 80 chemins au worktree : **80/80 identiques, aucun absent ni divergent**. La dernière lecture de commits sur la branche donne ce SHA.

## Incident de transport et correction

Le premier commit **383ee9b4e36674d9555a7b4426c35ac7a290acd5** a marqué les contenus comme Base64 alors que cette intégration les a stockés comme texte Base64 littéral. Le readback a immédiatement montré des tailles et blobs divergents; ce commit n’est pas réputé comme transfert réussi.

Un commit de correction **62352c4bb7f9a872136aab044a4d8d5d7a8aedf2** a d’abord remplacé un petit JSON en texte UTF-8; la taille relue est 274 octets et le blob correspond au local. Le commit **984a9385…** a ensuite réécrit en texte les 79 autres chemins, dont les gros fichiers. Le readback exact 80/80 sur le SHA final confirme le contenu courant. Les commits intermédiaires restent dans l’historique; aucun force-push ni réécriture d’historique n’a été fait.

## CI et accès

Le pipeline 172, lié au SHA final, est **success** : **pc4_gitleaks**, **pc4_semgrep**, **pc4_trivy** et **pc4_ci_fallback** sont tous en succès. Le pipeline 170 du commit erroné était aussi vert, ce qui n’avait pas détecté le problème de représentation; le readback des blobs a été nécessaire.

Le connecteur GitLab déjà authentifié a suffi. Aucun courriel n’a été lu, aucun PAT n’a été copié ou utilisé. Les opérations et tests Rust restent sur PC6; GitLab fournit le dépôt distant et la CI, pas un terminal de développement PC4. Aucun runtime AgentOS/LLM, merge, benchmark ou gain produit n’est démontré par ce transfert.

## Commandes et preuves

Les noms d’outils API, commits, pipeline et 80 paires chemin/blob/mode sont dans **commands.txt**, **pipeline.json** et **readback.json**. Le premier transfert Base64 et sa correction sont conservés dans l’historique GitLab sans être présentés comme un succès. Les résultats du lot produit CONTINUATION-436 sont dans [son reçu local](../continuation-436-leading-dash-branch-ref-001/local-run.md).
