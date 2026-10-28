# CONTINUATION-033 — assertion des requêtes HTTP du processus de réconciliation

Date : 26 octobre 2026. Périmètre : correction d’un test de sécurité du CLI dans le workspace local; aucun changement de la logique produit ou de Forgejo.

## Finding et correction

Une revue indépendante a relevé une lacune dans `actual_reconcile_cli_process_does_not_follow_redirect_with_host_credential`. L’ancien test comparait `requests.len() == 1` et `upstream_read_attempts == 1`, mais la fixture à `127.0.0.1:3130` s’arrêtait dès la première connexion. Son compte HTTP était donc borné par le harnais; cela ne prouvait pas qu’aucune seconde GET n’était envoyée au même endpoint.

La nouvelle fixture de ce test continue d’accepter et d’enregistrer les requêtes pendant toute la durée du vrai processus CLI. Le test lui envoie un signal d’arrêt après la fin du processus; la fixture draine ensuite jusqu’à 500 ms de silence, et répond à toute connexion reçue pendant cette fenêtre. Le compte `requests.len() == 1` porte ainsi sur les requêtes HTTP réellement reçues par le serveur, indépendamment du compteur interne. Le CLI s’exécute dans son groupe de processus et le helper existant tue le groupe si la limite de cinq secondes expire. L’observateur séparé du port `3131` reste actif et vérifie qu’aucune requête n’a suivi le 302.

## Vérifications

Le test ciblé est lancé directement sous Bubblewrap avec `--unshare-net`, en réservant les deux ports loopback dans l’espace isolé. Résultat conservé dans `focused-process.log` : 1 réussi, 0 échec, 0 ignoré, en 0,57 s. Le processus a observé une GET authentifiée au chemin API exact; `requests.len() == 1` passe. L’observateur du port de redirection ne voit aucune requête.

`cargo test --locked --offline --test webhook_receiver_process_e2e actual_reconcile_cli_process_does_not_follow_redirect_with_host_credential --no-run`, Clippy ciblé, `cargo fmt --all -- --check`, `git diff --check` et le garde de métadonnées Cargo passent. Clippy garde `-D warnings` et ajoute les allowances de lint déjà utilisées sur ce test ainsi que `type_complexity` pour la signature existante de `start_scripted_cli_http_fixture`; aucune règle du code ajouté n’est désactivée. Métadonnées inchangées : `license=null`, `license_file=null`, `publish=[]`, droits `not_assessed`.

## Limites

- La fixture est un serveur HTTP synthétique loopback, pas Forgejo; l’instance Forgejo locale n’est pas sollicitée.
- Le test reste ignoré par Cargo par défaut parce qu’il réserve `127.0.0.1:3130` et doit être invoqué sous le runner réseau isolé.
- La fenêtre silencieuse de 500 ms draine les demandes déjà émises après l’arrêt demandé. Le test ne prétend pas couvrir un processus étranger qui survivrait au CLI et parlerait bien plus tard.
- Cette preuve ne démontre ni AgentOS/LLM, ni une autorité de fusion, ni un gain comparatif.

Commandes, versions et empreintes sont dans ce dossier. Le test n’a pas touché aux sources Forgejo, à un service externe, à un secret réel ou à la production.
