# WEBHOOK-BRANCH-CREATE-PERCENT-ENCODING-001 — création de branche et ref échappée

Date : 25 octobre 2026. Le checkout local est `HEAD=f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` sur `local/WEBHOOK-005-canonical-links` et son worktree reste fortement modifié; aucun changement préexistant n’a été remplacé.

## Comportement exercé

Le test processus `actual_reconcile_cli_process_reads_new_branch_with_literal_escape_from_zero_before_push` envoie au vrai récepteur une livraison push HMAC synthétique pour la création de `feature/release/%2Fagent-proof`. `git check-ref-format --branch` confirme que ce nom de ref est valide. Le champ `before` vaut quarante zéros et `after` vaut `cccccccccccccccccccccccccccccccccccccccc`.

La réconciliation démarre le vrai CLI local à partir du journal accepté. Elle n’effectue qu’un GET sur `/api/v1/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2F%252Fagent-proof`: les slashes deviennent `%2F` et le caractère `%` littéral devient `%25`. La réponse loopback doit porter le nom exact original et le commit courant. Le CLI retourne `current_state_read`, `state=present`, le SHA attendu et l’URL canonique correspondante. Aucun header Authorization n’est envoyé et le journal reste identique octet pour octet.

Résultats : le scénario processus ciblé passe 1/1, avec 61 cas filtrés, sous Bubblewrap 0.9.0 et namespaces réseau/PID privés. La suite Rust hors ligne passe 510 tests, 0 échec et 61 ignorés en 34 groupes. `cargo fmt --all -- --check`, `git diff --check` et la validation Git de la ref passent. Le changement de code est limité à ce test processus et à son entrée dans le sélecteur de runner; le client de production avait déjà son encodeur de segment canonique.

## Métadonnées de licence Cargo

La relecture fraîche donne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected` et `redistribution_rights=not_assessed`. Le garde et ses sept tests passent. Le manifeste crate garde `publish=false`; aucun SPDX, `license_file`, droit de redistribution ou publication n’a été choisi. Cette preuve confirme l’état, elle ne constitue pas une décision juridique.

## Commandes et limites

Les commandes complètes sont dans [`commands.txt`](commands.txt); les résultats du cas ciblé et de la suite sont consignés dans [`test-result.txt`](test-result.txt) et [`suite-summary.txt`](suite-summary.txt). Le readback Cargo est [`cargo-license-readback.json`](cargo-license-readback.json); les hashes des sources et la toolchain sont associés.

Le serveur API est une fixture HTTP loopback et non une instance Forgejo. Ce lot prouve le chemin récepteur HMAC → inbox durable → client de réconciliation au processus ainsi que l’échappement non ambigu; il ne prouve pas que Forgejo v17 émet ce push exact ni qu’il accepte ce chemin via son serveur réel. Le nouveau cas est ignoré par la suite par défaut et doit être sélectionné via le runner pour obtenir l’isolation loopback. Aucun LLM, AgentOS, service externe, secret réel ou coût n’a été utilisé. Le résultat ne démontre aucun gain comparatif.
