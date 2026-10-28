# CONTINUATION-408 — audit local du transfert GitLab PC4

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Portée et résultat de l’audit

L’utilisateur demande de poursuivre le projet sur son GitLab self-managed PC4, dépôt `root/gitspace-forgejo`. Le remote `pc4-gitlab` est déjà configuré sans secret dans l’URL et `origin` reste Forgejo local. Une recherche Gmail ciblée a retrouvé des courriels d’accès, sans recopier leur contenu dans les artefacts et sans utiliser de valeur d’accès. Le PAT documenté précédemment vise d’autres projets; aucun credential projet limité à `root/gitspace-forgejo` n’a été confirmé.

Le script `audit_export.py` inspecte les fichiers modifiés suivis, stagés et non suivis non ignorés, puis les membres des archives ZIP de ces chemins. Il compte 14 830 chemins, 14 829 fichiers réguliers et 181 709 806 octets; 2 712 membres ZIP sont lus sans erreur et aucun lien symbolique n’est présent. Les motifs couvrent des formats connus de PAT GitLab/GitHub, clés OpenAI/AWS, clés privées PEM et URL HTTPS avec userinfo. Une seule correspondance `url_userinfo` apparaît dans `tests/__pycache__/test_mission_requirement_arbitration_cli_process.cpython-312.pyc`; le source correspondant n’a pas de correspondance. 125 chemins de cache générés (`__pycache__`, bytecode et assimilés) sont recensés. Ils ne sont pas supprimés, mais doivent être omis de l’export Git.

Les chiffres et chemins détaillés sans valeur de secret sont dans `export-scan.json`; les vérifications de transport et le test du source correspondant sont dans `preflight-summary.txt` et `source-canary-check.txt`.

## Connectivité et état distant

Sur PC6, le pair PC4 apparaît dans l’état Tailscale; la résolution MagicDNS n’a pas de réponse. HTTPS/443 échoue avant l’établissement HTTP, y compris lors du test direct de l’adresse Tailscale, et `git ls-remote` anonyme échoue lors de la résolution du nom. Aucun header d’autorisation n’a été envoyé. Aucune authentification, lecture de refs, fetch, staging, commit ou push n’a eu lieu; l’état du dépôt distant n’est donc pas établi. Le log de l’observation est `preflight-summary.txt`.

## Limites et prochaine action

Le scan reconnaît uniquement les formats indiqués; il n’a pas couvert les fichiers ignorés, la mémoire des processus, l’environnement, les courriels ni tout format arbitraire de secret. Il ne constitue pas une garantie d’absence de credentials. Le transfert complet doit refaire l’audit des chemins exacts retenus après que l’accès distant fonctionne. Lire les refs et l’état du dépôt d’abord, préparer ensuite une branche dédiée, garder la branche par défaut intacte et vérifier le readback avant de poursuivre sur GitLab.

Pour débloquer le transfert, rendre le DNS et HTTPS/443 de PC4 joignables depuis PC6 (ou fournir un endpoint HTTPS sûr équivalent). Aucun paramètre Tailscale, remote Git ou permission n’a été modifié. En attendant, les lots locaux restent indépendants et continuent.
