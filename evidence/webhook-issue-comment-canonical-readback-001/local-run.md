# WEBHOOK-ISSUE-COMMENT-CANONICAL-READBACK-001

Date : 2026-10-26. Travail local dans le worktree du produit, base Git examinée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le code et les documents de ce lot sont encore des modifications non commit. Aucun reset, nettoyage, commit, déploiement, accès externe, modèle payant ou donnée privée exportée.

## Constat et décision

Avant ce lot, un webhook Forgejo `issue_comment` ordinaire était réduit à sa ressource issue parente et le reconciler ne faisait que `GET /issues/{number}`. Cela confirmait l’issue mais ni la présence/absence, l’identité, le lien parent, ni le corps courant du commentaire. Le point de départ est documenté dans `src/webhook/reconcile.rs::resource_kind_for_event` et `parse_resource_ref`; l’adaptateur HTTP est `src/bin/forgejo-reconcile-local.rs::LocalLoopbackForgejoApi::read_current`.

La source officielle Forgejo v17.0.0 du scratch d’audit, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, fournit une lecture par ID à `routers/api/v1/repo/issue_comment.go::GetIssueComment`. Pour un commentaire ordinaire, `modules/structs/issue_comment.go::Comment` et `services/convert/issue_comment.go::ToAPIComment` exposent `id`, `body`, `updated_at`, `issue_url` et `html_url`; `models/issues/comment.go::{IssueURL,HTMLURL,HashTag}` lie le commentaire à l’issue et forme le fragment `#issuecomment-{id}`. Les fichiers upstream ciblés ne sont pas copiés dans le produit.

Le nouveau contrat distingue `ResourceKind::IssueComment` et transporte issue number + comment ID. Il exige une action `created|edited|deleted` et un ID positif; lit d’abord l’issue canonique, puis `/issues/comments/{id}`; lie la réponse à l’issue, à l’ID et aux deux URLs; retourne uniquement le corps lu dans l’API, étiqueté `untrusted_user_content`. Un 404 de commentaire est `absent` seulement pour un événement `deleted` après lecture valide de l’issue parente. Tous les autres 404/commentaires incomplets restent inconclusifs. Les commentaires de conversation PR gardent leur chemin existant par `GET /pulls/{number}`.

## Échec puis correction

Le nouveau test du vrai binaire CLI a d’abord renvoyé `inconclusive/upstream_forbidden` après la lecture réussie de l’issue : la politique `reconciliation_request_is_allowed` autorisait `/issues/{n}` et `/pulls/{n}`, mais pas la nouvelle route commentaire. Le code ajoute maintenant un seul chemin permis, `GET /repos/{owner}/{repo}/issues/comments/{id}`, avec ID positif canonique en base 10. Le test unitaire refuse POST, corps, ID zéro, zéros de tête, suffixe, autre dépôt et query string. Le processus CLI repasse alors et émet le snapshot du commentaire courant.

## Vérifications conservées

Toutes les commandes sont hors ligne/verrouillées pour Cargo et les processus qui l’exigent tournent dans Bubblewrap avec namespaces réseau/PID isolés.

- `cargo check --offline --locked --tests` : succès.
- `cargo test --offline --locked --test webhook_reconcile_contract` : 48 réussis.
- `cargo test --offline --locked --bin forgejo-reconcile-local` : 140 réussis.
- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e` : 3 réussis, 2 ignorés (deux tests statut déjà isolés par leur runner).
- `cargo test --offline --locked --test reconcile_config_contract` : 4 réussis.
- `cargo test --offline --locked --test reconcile_journal_lock_timeout` : 3 réussis.
- Le processus `actual_reconcile_cli_process_routes_issue_comment_to_issue_api` : 1 réussi sous Bubblewrap. Il relit l’issue #24 puis le commentaire #916 par les deux routes prévues, sans Authorization; la fixture API retourne `closed`, le corps canonique « canonical comment body », et `body_trust=untrusted_user_content`. Son corps webhook est différent et n’est pas la source de sortie.
- Le processus de non-régression `actual_reconcile_cli_process_routes_issue_comment_pr_conversation_to_pull_request_api` : 1 réussi sous Bubblewrap; il continue d’utiliser la route PR.
- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e tests.test_forgejo_webhook_native_replay_e2e` : 122 réussis; les assertions de clients ont été mises à jour pour le schéma de batch 6 et single 5.
- `python3 -m py_compile` des quatre drivers et deux modules tests modifiés, `cargo fmt --all -- --check`, `git diff --check`, `python3 -m unittest scripts.test_cargo_license_state` et `python3 scripts/verify_cargo_license_state.py` : succès. Le readback Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.

Les sorties de ces commandes et les empreintes des sources sont jointes à ce dossier. Les schémas JSON de sortie sont single 5 / batch 6 et se découvrent via `--capabilities`.

## Portée et limites

Le chemin CLI complet utilise une vraie inbox/lecture d’arguments/binaire de production locale et une fixture HTTP loopback dans la namespace réseau privée; le serveur amont n’est pas Forgejo dans ces deux nouveaux processus. La forme des champs de commentaire a été confrontée au source officiel Forgejo v17.0.0 au commit ci-dessus et un précédent lot a déjà observé `GET /issues/comments/{id}` après DELETE sur une instance locale; ce rapport ne revendique pas une nouvelle lecture d’un commentaire vivant par l’adaptateur Rust. `body_trust` est une indication pour le consommateur, pas une sanitation et ne bloque pas une instruction hostile. Le résultat n’est ni un parcours LLM/AgentOS ni une comparaison de valeur, et `n=1` ne mesure aucune amélioration générale.
