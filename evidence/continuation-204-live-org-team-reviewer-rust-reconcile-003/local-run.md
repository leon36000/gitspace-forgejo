# CONTINUATION-205 — refus d’écriture du credential de réconciliation

Run local isolé du 27 octobre 2026 dans le worktree basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le checkout reste fortement modifié et aucun commit n’a été créé. Il réutilise le profil fixed `org-team-review-e2e` de CONTINUATION-204 et crée un nouvel état Forgejo jetable sous `/tmp/forgejo-team-review-cont204-state-003`.

## Résultat observé

Sur l’organisation privée `forge-team-org`, le dépôt privé `team-review-readback` contient une PR #1 ouverte avec l’équipe reviewer `reviewers` (`id=2`). Le token d’essai, créé pour le membre reviewer avec `read:user`, `read:repository` et `read:issue`, est d’abord utilisé par le vrai CLI Rust pour relire le webhook `pull_request/review_requested` et son état canonique.

Avant ce readback Rust, le harnais envoie avec le même token un unique `POST /repos/forge-team-org/team-review-readback/issues/1/comments` contenant un marqueur aléatoire sans données privées. Forgejo répond **HTTP 403**. Un GET administrateur de la liste des commentaires avant et après retourne `[]`; le marqueur n’est pas présent. Le fingerprint de PR avant/après est identique : numéro, titre/corps fixture, état, merge flag, base et tête, dépôt de base, listes de reviewers et `updated_at`. Le reçu `write-scope-probe.json` ne contient aucun token.

Après le refus, le receiver Rust a journalisé la livraison webhook native et `forgejo-reconcile-local` retourne `current_state_read`, avec la liste de teams reviewers exacte `[{"id":2,"name":"reviewers"}]`; l’inbox reste inchangée par la lecture. Les processus Forgejo et receiver terminent avec code 0, et aucun listener ne subsiste sur 3130 ou le port receiver. Reçu complet : `result.json`, `write-scope-probe.json`, `cleanup.json`.

## Tests et construction

Le test-first du validateur échoue d’abord car la fonction dédiée manque; après correction, `test_forgejo_live_org_team_reviewer_readback.py` passe **8/8**. `py_compile`, `cargo fmt --all -- --check`, `git diff --check`, le garde de métadonnées Cargo et ses **12 tests** passent. Cargo reste sans choix de licence (`license=null`, `license_file=null`, `publish=[]`, `NOASSERTION`, droits non évalués).

Le runner compile le vrai receiver Rust et le CLI Rust dans Bubblewrap 0.9.0 avec réseau/PID non partagés, source et dépendances staged read-only, et Cargo offline/verrouillé. Versions, hashes des exécutables et arbres source sont dans `build-provenance.json`. Forgejo est `17.0.0+gitea-1.22.0`, Go 1.26.9; le commit exact du binaire est inconnu.

## Portée et limites

C’est une seule instance locale (`n=1`) et une seule route write non destructive. Elle montre que Forgejo v17 refuse cette écriture pour ce token/scopes sur cette fixture et que la lecture de PR continue à fonctionner. Elle n’établit pas le refus de toutes les routes write, des transports Git, d’autres tokens/scopes ou des autres versions/configurations; elle n’est pas une frontière de sécurité générale ni un audit exhaustif des permissions. Aucun agent/LLM/AgentOS, merge, validation post-merge ou benchmark comparatif. Aucun code upstream, coût, service externe, production, publication ou commit n’est impliqué. Les états antérieurs et celui-ci restent préservés.
