# CONTINUATION-437 — compatibilité d’un tag `@` — 28 octobre 2026

Worktree : branche `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt déjà fortement modifié. Le lot a ajouté un cas au test d’intégration Release; aucun runtime, historique Git ou service n’a été modifié.

## Résultat

L’inspection de `valid_tag_ref_name` a suggéré que `@` pouvait être refusé comme ref spéciale. Le test exploratoire ajouté au jeu des noms invalides a échoué parce que la réconciliation acceptait `@`. L’oracle local `git check-ref-format refs/tags/@` (Git 2.43.0) retourne pourtant le code 0 : le nom utilisateur est `@`, tandis que la ref Git complète contient le namespace `refs/tags/` et est valide. Il n’y avait donc pas de défaut runtime à corriger.

Le cas a été déplacé vers le test positif `release_accepts_git_valid_unicode_whitespace_and_at_current_tag_names`. Ce test passe par la réconciliation d’une notification `release`, relit un snapshot API de fixture avec `tag_name="@"`, et vérifie le tag conservé ainsi que l’URL canonique `/releases/tag/@`. L’encodeur Forgejo-compatible laisse `@` non échappé dans ce segment. Aucun code runtime n’a changé.

## Vérifications

- `git check-ref-format refs/tags/@` : code 0.
- Test du cas positif : 1/1.
- `cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1` : 104 réussis, 0 échec, 0 ignoré.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 765 réussis, 0 échec, 200 ignorés, 38 groupes. Les ignorés ne sont pas comptés comme exécutés.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py` et `cargo metadata --format-version 1 --locked --offline --no-deps` : code 0.
- L’audit des liens dans les cinq documents canoniques relit 1 607 liens, sans cible manquante.
- Le garde de licence confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, marqueur `NOASSERTION`.

Les versions, sorties et empreintes sont jointes à ce dossier.

## Limites

Le readback Release est une fixture HTTP locale; aucun serveur Forgejo n’a été lancé. La vérification Git couvre un nom ponctuel avec Git 2.43.0 et n’établit pas une conformité exhaustive aux autres versions ou forks. Aucun LLM, AgentOS, RAG, mémoire, GitLab, courriel ou service externe n’a été utilisé. Ce lot ajoute une garde de compatibilité au contrat de réconciliation, sans changement de comportement runtime; il ne démontre pas un parcours agent ou un gain de productivité.
