# WEBHOOK-REDIRECT-PROCESS-001 — refuser une redirection dans le chemin receiver → CLI

Date locale : 25 octobre 2026. Dépôt de travail : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste fortement modifié; les changements préexistants ont été conservés.

## Résultat

Un test processus ignoré par Cargo par défaut, `actual_reconcile_cli_process_does_not_follow_redirect_with_host_credential`, traverse le vrai binaire receiver, son inbox, puis `forgejo-reconcile-local`. Le webhook, le secret HMAC et le token sont synthétiques. Après admission, le CLI envoie le token par pipe privé à l’origine loopback autorisée `127.0.0.1:3130`. La fixture répond `302 Location: http://127.0.0.1:3131/credential-must-not-follow`.

Le résultat est `inconclusive`, code `upstream_http_status`, statut 302 non retryable, une tentative amont et aucun état canonique. La fixture d’origine observe une seule requête GET avec l’Authorization attendu; le serveur de redirection sur le port 3131 n’observe aucune connexion. Le journal contient toujours exactement l’événement admis. Aucun vrai credential ou service externe n’a été utilisé.

Le test est d’abord devenu rouge lorsqu’une mutation temporaire ajoutant `curl --location` à `src/bin/forgejo-reconcile-local.rs` a été introduite : le serveur du port 3131 a reçu `GET /credential-must-not-follow`. La mutation a ensuite été retirée; le test final vert confirme le refus de ce chemin. Le journal rouge montre aussi que curl n’a pas retransmis le header Authorization à ce port différent dans ce cas. Le test établit l’absence de requête redirigée et l’absence de succès canonique, pas une règle générale de transmission de credentials pour toutes les versions/options curl.

## Isolation du runner

Avant ce lot, `scripts/run_webhook_reconcile_cli_e2e.py` compilait la cible Cargo hors sandbox puis montait la racine `/` hôte en lecture seule. Ce runner a été remplacé par une délégation aux helpers allowlistés de `scripts/run_forgejo_mission_delivery_process_e2e.py`: le dépôt est copié sous forme d’entrées Cargo bornées, les crates du lock sont copiées depuis le cache hors ligne, Cargo et les build scripts compilent dans Bubblewrap, et seul le binaire de test ainsi que les binaires explicitement nommés sont exportés. Le processus E2E tourne avec `--unshare-net`, `/home` et `/tmp` en tmpfs, le workspace en lecture seule et `.runtime` en tmpfs. Aucun bind de la racine ou du home hôte n’est dans la commande finale. Le test Python `test_webhook_runner_delegates_build_and_execution_to_allowlisted_runner` vérifie cette délégation et passe.

Le scénario sélectionné passe 1/1 dans Bubblewrap 0.9.0; 21 autres cas du binaire de test ont été filtrés. Le contrôle de runner passe 1/1. Vérifications finales : `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py scripts/test_webhook_reconcile_runner_sandbox.py` sortent avec code 0. Les logs du test isolé, du contrôle Python et des commandes sont archivés ici.

## Cargo et droits

`python3 scripts/verify_cargo_license_state.py` relit `cargo metadata --locked --offline --no-deps`: `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected` et `redistribution_rights=not_assessed`. Les cinq tests de politique passent. Aucune licence n’a été choisie et aucune conclusion sur les droits de redistribution n’est ajoutée.

## Portée et limites

Le test valide la frontière du client HTTP Rust avec des fixtures loopback privées. Il ne teste pas une instance Forgejo, une pull request, un workflow Actions/runner, une vérification indépendante, une fusion, un LLM, AgentOS, le RAG ou la mémoire. Le test n’est pas exécuté automatiquement par `cargo test --all-targets`; il est invoqué ici par le runner isolé. Ce résultat n’est pas le parcours complet ni une mesure de gain.

Aucune source Forgejo ou GitSpace n’a été copiée dans le produit. Aucun changement de production, publication, suppression, dépense ou extension d’accès n’a eu lieu. Le manifeste local couvre 11 artefacts et passe 11/11. Après ajout de ce lot, le manifeste global compte 2 698 lignes : 2 697 entrées SHA-256 valides, présentes et concordantes; son unique ligne historique non hashée est préservée. Les versions et empreintes de sources, les logs rouges/verts et le manifeste local sont dans ce répertoire.
