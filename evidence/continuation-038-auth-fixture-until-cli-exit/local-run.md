# CONTINUATION-038 — compter indépendamment les lectures API authentifiées

Date : 26 octobre 2026. Worktree local `.runtime/webhook-005-workspace`, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les changements du worktree restent non commités et le fichier de test lui-même est non suivi dans ce checkout. Aucun appel réseau externe, serveur Forgejo, modèle, ou modification de production n’a été utilisé.

## Lacune et changement

Le test processus `actual_reconcile_cli_process_uses_host_credential_fd_for_receiver_journal` utilisait un serveur de fixture qui traitait une connexion puis s’arrêtait. Il affirmait aussi un seul accès à partir de `upstream_read_attempts`, valeur produite par le CLI testé. Ce n’était pas une observation indépendante de tous les accès HTTP reçus.

Les trois tests concernés utilisent maintenant `start_authenticated_cli_http_fixture_until_finished`, qui accepte et enregistre les requêtes jusqu’à ce que le processus CLI soit terminé et que le test signale la fin. Le test de credential valide exige un seul GET vers l’issue configurée et un en-tête autorisé. Le test de credential invalide exige un seul GET, un refus `upstream_unauthorized` et aucun credential dans stdout/stderr. Le test batch exige un seul GET pour l’événement dans le dépôt configuré, aucun GET pour l’événement hors scope, et aucun credential dans stdout/stderr. Chaque test vérifie le compte observé par le fixture, indépendamment de `upstream_read_attempts`. Les trois processus sont lancés dans leur propre groupe et une échéance de cinq secondes borne leur attente. L’ancien helper qui quittait après une seule connexion a été retiré.

## Vérification

- `cargo test --locked --offline --test webhook_receiver_process_e2e --no-run` : compilation réussie.
- Les trois tests ignorés par défaut ont été lancés explicitement, chacun dans Bubblewrap 0.9.0 avec `--unshare-net`, sur loopback uniquement : 3/3 réussis, 0 échec, 84 filtres par scénario.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Cargo indique `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`.
- Les commandes, versions et SHA-256 de la source du test figurent dans les fichiers voisins. Les index locaux et globaux valident les dernières empreintes des chemins de cette tranche et conservent leurs préfixes historiques.

La première vérification `rustfmt --check` a signalé deux lignes de mise en forme dans la modification du test; le format a été corrigé et le contrôle final passe. Le journal initial est gardé avec les autres traces.

## Portée et limites

Le CLI Rust réel lit des événements locaux et transmet des tokens synthétiques par pipe; la destination HTTP est une fixture, pas Forgejo. La collecte reste bornée à la durée de vie du CLI et ne teste pas un processus externe hostile qui se détacherait. Aucun test de mutation n’a injecté une seconde requête; le nouveau compte indépendant la refuserait si elle était reçue avant la fin du CLI. Cette tranche renforce des contrats de test et ne modifie pas la logique d’exécution du produit.

Le parcours LLM/AgentOS, la comparaison Forgejo+agent+CI, la précondition obligatoire pour tous les writers Forgejo, et la récupération LFS opérationnelle restent non démontrés. Cette preuve isolée ne justifie aucune conclusion de gain général; le projet demeure incomplet.
