# FORGEJO-BRANCH-QUERY-ALLOWLIST-001 — refus des query strings sur les routes branche

Date : 25 octobre 2026. Checkout local, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, arbre de travail fortement modifié. Le diff produit de ce lot est local; aucun commit, push, publication ou changement production.

## Constat et correction

La politique `reconciliation_request_is_allowed` acceptait `GET /repos/<owner>/<repo>/branches/<branch>?format=raw`, car `encoded_branch_path_is_valid` retirait le suffixe `?…` avant de vérifier le chemin. Le contrôle de scope faisait la même normalisation. Cela permettait à une forme de requête hors contrat de passer le filtre avant la lecture du credential et l’appel curl.

Un test d’abord rouge appelle la vraie méthode `request_api_once` avec un fournisseur de credential compteur et une fixture HTTP. Il exige `Forbidden`, zéro lecture du credential et zéro requête servie. Avant le correctif, il échoue sur l’attente `Forbidden` (exit 101). Le fix rejette `?` dans la validation d’une route branche et vérifie le chemin original au contrôle de scope. Le test vert couvre les routes `branches/main?format=raw` et `branches/feature%2Frelease%2Fagent-proof?format=raw`; les deux sont refusées avant toute lecture et connexion.

## Vérifications

- Test ciblé après correctif : 1/1 réussi; log rouge avant correctif et log vert après correctif conservés.
- `cargo test --offline --locked --bin forgejo-reconcile-local -- --test-threads=1` : 117 réussis, 0 échec, 0 ignoré.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 510 réussis, 0 échec, 59 ignorés, 34 groupes.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_current_branch_after_stale_push_webhook` : 1 réussi, 59 filtrés, sous Bubblewrap. Ce scénario régressif confirme qu’une route de branche imbriquée sans query string conserve son encodage et la tête API courante.
- `cargo clippy --offline --locked --bin forgejo-reconcile-local --tests` : exit 0; un avertissement `clippy::too_many_arguments` reste dans le helper de fixture `ProfileGateFixture`, hors du changement.
- `cargo fmt --all -- --check` : premier contrôle rouge pour le format du test ajouté; correction appliquée, contrôle final exit 0.
- `git diff --check` : exit 0.
- La garde Cargo de licence est vérifiée : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`; ses 7 tests passent.

Les commandes complètes et toolchain sont dans [`commands.txt`](commands.txt) et [`toolchain.txt`](toolchain.txt). Journaux : `red.log`, `green.log`, `bin-tests.log`, `all-targets.log`, `branch-process-regression.log`, `clippy.log`, `clippy-final.log`, `rustfmt.log`, `rustfmt-initial-failure.log`, `rustfmt-final.log`, `diff-check.log`, `diff-check-final.log`, `license-guard.log` et `license-tests.log`.

Le manifeste local `SHA256SUMS` couvre ces artefacts; `global-current-entries.txt` permet de vérifier les empreintes actuelles des sources, documents et preuves depuis la racine du worktree.

## Portée et limites

Le test utilise un credential entièrement synthétique et un serveur loopback; aucun Forgejo live, secret réel, modèle, AgentOS, RAG ou service externe n’a été appelé. Le refus est un contrôle du client d’adaptateur. Il ne constitue pas une politique réseau OS et n’empêche pas un autre processus de joindre directement Forgejo. La branche est actuellement validée dans le client HTTP du profil; le prochain travail de sécurité reste l’intégration de l’exécuteur dans un sandbox réel puis la séparation vérifiée des capacités gate/vérificateur/exécuteur.
