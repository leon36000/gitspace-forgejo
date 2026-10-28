# CONTINUATION-246 — partage du parent récupéré entre `issues` et `issue_comment`

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, commit, publication, appel externe, modèle ou service de production.

## Scénario

Le test `actual_reconcile_cli_process_shares_recovered_parent_issue_read_with_comment_batch` fait accepter par le vrai receiver deux livraisons HMAC distinctes, séquences 1 et 2 : une modification d’issue puis un commentaire d’issue sur le même dépôt et numéro. Le CLI Rust réel traite les deux IDs dans un batch. Une fixture HTTP loopback répond d’abord `429 Retry-After: 0`, puis `200` avec l’issue courante `open`, puis `200` avec le commentaire courant.

Le processus effectue exactement trois GET, dans cet ordre : issue 429, issue 200, commentaire 200. Le résultat parent récupéré est réutilisé pour l’événement `issue_comment`; le commentaire garde sa lecture distincte. Les deux lignes sont `current_state_read`; le commentaire est `present`, son texte vient du readback et sa confiance reste `untrusted_user_content`. Le test vérifie les IDs, l’ordre, les chemins, l’absence de `Authorization`, l’absence du corps 429 et du texte webhook dans stdout/stderr, et l’égalité octet pour octet du journal avant/après.

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_shares_recovered_parent_issue_read_with_comment_batch`. Le runner compile hors ligne puis exécute le binaire de test sous Bubblewrap `--unshare-net --unshare-pid`; résultat `1 passed; 0 failed; 0 ignored; 150 filtered out`. Deux invocations isolées du même scénario ont passé; la sortie intégrale de l’invocation archivée est `test.log`.

## Changement et portée

Ce lot ajoute une preuve processus au comportement existant : le diff de code est limité à ce test et à son entrée dans l’allowlist du runner. Aucun code runtime Rust ou `Cargo.toml` ne change. Le cas étend CONT-244 (deux événements `issues`) au partage du parent entre événements `issues` et `issue_comment`; il ne réaffirme pas un gain de débit ni un snapshot atomique.

L’API est une fixture loopback scriptée; ce scénario précis n’interroge pas un serveur Forgejo. La suite générale Rust n’a pas été lancée. Le résultat ne couvre pas concurrence ou mutation de l’issue entre les requêtes, ni tout événement de commentaire ou toutes réponses Forgejo. Aucun LLM/AgentOS n’est utilisé.

## Métadonnées Cargo sans licence choisie

Readback frais via `scripts/verify_cargo_license_state.py` (`cargo metadata --locked --offline --format-version 1 --no-deps`) : paquet `agent-forge-core`, `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, redistribution `not_assessed`, marqueur personnalisé `NOASSERTION`. Les 12 tests de la garde passent. Cette métadonnée n’accorde aucun droit et ne sélectionne pas la licence produit ni celle des sources tierces.

Formatage final `cargo fmt --all -- --check`, `git diff --check`, AST Python du runner et contrôle d’espaces terminaux passent. La vérification initiale de formatage a seulement demandé la mise en forme rustfmt du nouveau tableau; `cargo fmt --all` l’a appliquée avant le run archivé.

Rapport, sortie du runner, commandes, versions, hash des sources et index append-only : ce dossier. Le checkpoint et la matrice sont mis à jour. La mission complète reste ouverte : aucun parcours LLM/AgentOS, benchmark contrôlé ou gain général n’est démontré.
