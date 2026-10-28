# CONTINUATION-331 — conflit d’idempotence après changement de labels seuls

Date : 2026-10-28. Dépôt : worktree local fortement modifié, branche `local/WEBHOOK-005-canonical-links`, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Seule la fixture de test processus a été étendue dans cette tranche; le comportement produit n’a pas été modifié.

## Scénario et résultat

Le test conserve le claim initial et son retry idempotent. Puis l’API fixture sert une issue dont seuls les labels changent (ID 7 → ID 12); titre, corps et `updated_at` restent constants. Le webhook signé `issues/edited` apporte volontairement un ancien titre/corps, et le receiver l’accepte à la séquence 2. Le vrai `forgejo-mission-local`, exécuté dans la composition Bubblewrap existante, relit l’API et retourne `inconclusive/execution_idempotency_conflict`. Les octets du journal de mission et du fichier `host_admission` sont égaux à leurs snapshots après le premier claim. L’édition de titre/corps préexistante poursuit ensuite à la séquence 3 et garde les mêmes refus.

La fixture compte 51 requêtes, dont 13 GET d’issue, 13 lectures de branche, 13 lectures de la spécification protégée, 4 lectures des paramètres API, 4 arbres Git et 4 lectures du fichier source. La seule livraison nouvelle du test est artificielle et locale.

## Commandes exécutées

- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation` — PASS, 1 test processus exécuté sous Bubblewrap; sortie complète : [process-e2e.log](process-e2e.log).
- `cargo fmt --all -- --check` — PASS : [rustfmt.log](rustfmt.log).
- `python3 scripts/verify_cargo_license_state.py` — PASS; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution non évaluée : [cargo-license-state.json](cargo-license-state.json).
- `git diff --check` — PASS pour le diff suivi : [tracked-diff-check.log](tracked-diff-check.log).
- Runtime observé par le runner : Rust/Cargo 1.95.0, Bubblewrap 0.9.0, Linux aarch64, tests offline. Le réseau a été désactivé dans l’espace d’exécution du processus.

## Migration GitLab demandée

L’utilisateur a demandé de transférer le projet vers son GitLab auto-hébergé à `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` et a indiqué que l’accès figure dans son courriel. Le checkpoint précédent indique que ce sont des accès initiaux à renouveler à la première connexion. Aucun secret n’a été relu, utilisé ou placé dans un fichier/une commande dans ce lot.

Le pair PC4 répond à `tailscale ping --c 1 --timeout 3s pc4`, mais `getent ahostsv4 pc4.tail0c55ff.ts.net` ne renvoie aucune adresse. Une connexion TCP directe, via l’adresse Tailscale issue de `tailscale status --json`, expire sur 22, 80, 443, 5050 et 8080. En conséquence, aucune authentification, aucun `git ls-remote`, aucune lecture de refs distantes, aucune modification Git locale et aucun octet de dépôt n’a été envoyé. Il faut rendre HTTPS/SSH accessible depuis ce poste puis lire les refs avant de préparer une branche de migration sans écrasement.

À la mesure précédente, le répertoire de travail occupait environ 22 Go, surtout `target/` et `.runtime/`; Git les ignore. La liste Git non ignorée comprenait environ 13 377 fichiers / 168 Mo avant l’ajout du présent reçu. Aucun snapshot de migration ni scan complet de secrets n’a encore été fait. Avant tout push, auditer les exclusions générées et secrets; ne pas pousser les caches ou secrets.

## Portée et limites

Ceci est une vérification du conflit d’idempotence en fixture locale; elle ne prouve ni intégration Forgejo live, ni exécution AgentOS/LLM, ni travail accompli par un modèle, ni décision/fusion indépendante ou observation post-fusion. Elle ne mesure ni gain, tokens, coût, interventions humaines ou temps de livraison. Le transfert GitLab reste bloqué au réseau avant authentification. Le projet demeure incomplet.
