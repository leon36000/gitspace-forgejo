# CONTINUATION-352 — refus d’un readback d’assignees PR malformé

Date : 2026-10-28. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le changement ajoute un scénario au test de processus et au runner isolé; aucune logique de production n’a été modifiée.

## Parcours du test

Le receiver local admet un `pull_request/synchronized` signé avec un assignee périmé. Le binaire réel `forgejo-reconcile-local` lit le journal puis effectue un seul GET pour le pull request 48. La fixture HTTP répond avec un snapshot autrement valide, mais contient deux assignees avec le même ID.

Le processus termine avec code 1 et rend `verdict=inconclusive`, `error.code=invalid_current_state`, `details.field=assignees` et `retryable=false`. Il ne retourne aucun `canonical_state`; l’identifiant périmé de l’événement n’apparaît pas dans la sortie, le journal reste identique octet pour octet et la requête ne contient pas d’en-tête Authorization. Le scénario tourne dans Bubblewrap avec `--unshare-net`; le serveur est une fixture loopback jetable, pas Forgejo live.

Commande reproductible :

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_malformed_current_pull_request_assignees
```

Résultat archivé dans [`isolated-test.log`](isolated-test.log) : 1 réussi, 0 échec, 171 filtrés. La suite contrat [`contract-tests.log`](contract-tests.log) passe 91/91.

## Vérifications complémentaires

[`local-checks.log`](local-checks.log) archive `cargo fmt --all -- --check`, `git diff --check`, analyse AST du runner, vérification des métadonnées de licence et ses 12 tests. Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed` et SPDX `NOASSERTION`.

## État du transfert GitLab demandé

La cible est `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`; l’utilisateur a indiqué que les accès sont dans son courriel. Le contrôle non authentifié archivé dans [`network-check.txt`](network-check.txt) trouve pc4 en ligne et pingable via Tailscale, mais le nom MagicDNS ne résout pas et les ports TCP 22, 80, 443 et 8443 expirent. HTTPS en imposant la résolution via le pair et le nom TLS attendu expire après 3 secondes. Aucun secret n’a été utilisé, et aucune ref distante n’a été lue, aucun login, commit de migration ou push n’a eu lieu. `origin` reste le serveur Forgejo local `http://127.0.0.1/forge-admin/agent-forge-core.git`.

Le courriel déjà examiné précise que le mot de passe root initial demande une rotation lors du premier login Web. Il n’a pas été utilisé ni changé. Il faudra que l’instance écoute sur son interface Tailscale et que son ACL/pare-feu accepte PC6 sur HTTPS 443 ou SSH 22. Après rétablissement, lire les refs d’abord, auditer les fichiers exportables, puis utiliser une branche de migration `codex/` sans force-push; exclure `.runtime`, caches non référencés, secrets, credentials et données privées étrangères au projet.

L’audit expurgé courant est [`transfer-content-audit-current.json`](transfer-content-audit-current.json), produit par le scanner local dérivé de CONT-336 et archivé avec sa commande dans [`audit_transfer_contents_current.py`](audit_transfer_contents_current.py) et [`transfer-audit-run.log`](transfer-audit-run.log). Il couvre 19 fichiers suivis modifiés et 13 766 chemins non suivis non ignorés (13 785 chemins candidats, 13 784 fichiers ordinaires; un chemin est un dépôt Git imbriqué). L’arbre `evidence/` fait 380 079 765 octets; les 28 ZIP représentent 107 151 276 octets et 2 712 entrées. Il n’y a aucun lien symbolique, fichier supérieur à 20 MiB, nom de credential, motif connu de secret, affectation littérale détectée ni erreur de lecture ZIP. Résultat heuristique seulement, pas une garantie d’absence de secret.

Avant transfert, conserver les 28 archives ZIP et les fichiers référencés par les manifestes; seules les 100 entrées `.pyc` non référencées sont des caches à exclure (121 fichiers `.pyc` totaux, 21 référencés). Ne pas copier le dépôt bare imbriqué; son bundle de 304 octets est vérifié et conservé dans l’évidence. Exclure `.runtime` ignoré et `.claude/ralph-loop.local.md`, qui contient un identifiant de session locale et des instructions propres à un run terminé. Les fichiers d’évidence utiles restent dans le snapshot demandé.

## Limites

Ce test prouve le refus d’une liste d’assignees API malformée au niveau du processus CLI; il ne teste pas Forgejo live, les droits d’un compte, GitLab, une fusion, AgentOS ou un modèle. Le réseau bloque le transfert demandé. Le parcours LLM/AgentOS et toute comparaison de valeur restent non démontrés; la mission reste incomplète.
