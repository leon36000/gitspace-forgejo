# CONTINUATION-153 — invariant d’ordre du readback batch de commentaires

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucun état antérieur n’a été supprimé, commit ou publié.

## Couverture

Le test processus ajouté dans CONTINUATION-152 a été étendu à deux sélections des mêmes trois IDs de livraison : issue → `created` → `deleted`, puis `deleted` → `created` → issue. Le receiver avait accepté ces événements une seule fois et dans l’ordre issue → création → suppression; seul l’ordre des IDs remis au CLI diffère. Chaque sélection lance le vrai `forgejo-reconcile-local` sous le runner Bubblewrap `--unshare-net --unshare-pid`, avec un nouveau fixture HTTP loopback pour le parent issue (`200`) et le commentaire (`404`).

Les deux permutations passent dans un test ciblé : **1 test réussi**, 0 échec, 120 filtrés. Chacune produit exactement deux GET : `/issues/24`, puis `/issues/comments/916`. Les deux invocations n’émettent pas d’Authorization, ne divulguent ni le corps de `404` ni le commentaire non fiable, et laissent le journal identique octet pour octet. Les trois résultats restent liés à leur ID : issue `current_state_read`, création `inconclusive/resource_not_found`, suppression `current_state_read` avec `comment.state=absent`. La sortie de chaque invocation rapporte deux lectures amont.

Le test vérifie ainsi que l’interprétation événementielle d’un 404 partagé ne dépend pas de l’ordre de traitement du batch. La correction de cache est antérieure et reste décrite dans [CONTINUATION-151](../continuation-151-cross-action-comment-cache-001/local-run.md); cette tranche ne modifie pas le runtime.

## Vérification

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_shares_current_comment_read_across_create_delete_batch`: **exit 0**, un test qui exécute deux permutations.
- `rustfmt --edition 2024 tests/webhook_receiver_process_e2e.rs` puis `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs`: réussis.
- Parsing AST du script runner, `git diff --check`, garde d’état Cargo et vérification append-only : réussis; les reçus sont dans ce dossier.

## Limites

Le receiver n’a pas reçu une seconde séquence d’événements inversée : le test inverse l’ordre de sélection au CLI, et le contrat unitaire couvre séparément l’ordre inverse des événements réconciliés. La fixture ne constitue pas un serveur Forgejo réel. Aucun LLM/AgentOS, secret réel, coût, accès externe ou benchmark n’a été utilisé. Ce résultat n’établit pas de gain général; le projet reste incomplet.

Suite : auditer la matrice des preuves et choisir un manque distinct et utile; ne pas rejouer ce batch.
