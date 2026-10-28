# CONTINUATION-268 — lier les sources aux refus batch d’attestation

Date : 27 octobre 2026. Worktree local fortement modifié, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, nettoyage, commit, publication, suppression ou changement de production. Les manifestes locaux et globaux sont append-only; les préfixes antérieurs sont capturés dans `manifest-prefix-before.json`.

## Comportement ajouté

Après résolution complète de tous les IDs demandés, les refus batch structurés conservent désormais `selected_event_sources` quand ils n’ont pas de lignes `results`. Le helper commun couvre les dispositions de quarantaine invalides, la politique de clés d’attestation absente, les attestations absentes/non approuvées/invalides, et toute erreur d’enveloppe retournée par la réconciliation batch. Il lie, dans l’ordre demandé, l’ID à `journal_source.sequence`, `event_sha256` et à l’observation d’attestation. Il n’inclut pas la signature, un résultat par livraison ou un état canonique. Les sélections impossibles à résoudre entièrement restent refusées avant ce champ; une erreur de lecture du pipe credential remonte encore comme erreur CLI non structurée et ne l’expose pas.

Le contrat batch passe de v15 à v16, le contrat capacités de v13 à v14; single reste v13. Le contrat précise que cette liaison de sources ne prouve ni l’authenticité du writer local ni la vérité amont.

## TDD et vérification

Un test unitaire rouge reproduit le trou : deux événements déjà sélectionnés (un journalisé avec signature receiver, l’autre legacy), credential FD fourni, aucune clé de confiance; le refus `host_journal_attestation_policy_missing` ne contenait pas `selected_event_sources`. Le run ciblé a échoué uniquement sur l’absence attendue de cette liste. Après ajout du rattachement, le test passe : séquences/hashes et IDs concordent, statuts `present_unverified`/`missing`, signature omise, zéro état canonique et refus avant credential/API.

Le test processus existant `actual_reconcile_cli_process_requires_trusted_attestation_policy_before_host_credential_read` a été étendu à deux livraisons signées réellement émises par le receiver local. Exécuté sous Bubblewrap via `run_webhook_reconcile_cli_e2e.py`, le CLI réel refuse faute de clé de confiance, restitue les deux ID→séquence/hash avec `present_unverified`, ne révèle pas le token synthétique, laisse les octets du pipe credential disponibles et ne modifie pas le journal. Résultat : 1/1, 157 tests filtrés.

Le test de quarantaine invalide vérifie désormais que le refus batch conserve les références des deux livraisons actives; les cas single restent au schéma v13. Les contrats capacités, configuration, verrou de journal et quarantaine passent respectivement 5 réussis/2 ignorés, 4, 3 et 5. Les trois modules Python consommateurs passent 190 tests. Vérification finale : `cargo test --locked --offline --all-targets` = 687 réussis, 0 échec, 144 ignorés sur 37 blocs; `cargo check --all-targets`, rustfmt, `py_compile`, `git diff --check`, garde Cargo et ses 12 tests passent.

Les diagnostics sont conservés : `rust-red.log` est le test rouge attendu; le premier `cargo-contract-tests.log` échouait sur une attente batch v15 oubliée, corrigée à v16; le premier `cargo-fmt-check.log` signalait uniquement un repli de ligne et `cargo fmt --all` l’a corrigé. Les sorties finales sont séparées. Les versions, commandes, sources et hashes sont inclus dans ce dossier.

Les deux scripts de scellement de manifeste ont aussi passé `python3 -m py_compile`. Les deux fichiers `.pyc` produits par ce contrôle restent conservés sous `__pycache__/` et seront indexés dans le manifeste local avec les autres pièces du dossier.

## Portée et réserves

C’est une vérification locale avec le receiver Rust, le CLI Rust, une fixture HTTP/Bubblewrap et des tests contractuels; aucune instance Forgejo live, AgentOS ou LLM n’a été utilisée. La signature facultative observe seulement la provenance du receiver selon la politique de clés fournie; `present_unverified` reste non vérifié, le HMAC source brut n’est pas revérifié au rejeu, et un writer local capable de réécrire le journal n’est pas authentifié. La nouvelle liste ne couvre pas encore les échecs de lecture du FD credential non structurés, les IDs absents/dupliqués refusés avant sélection complète, ni l’exclusion de writers externes pendant le merge. Aucun gain comparatif n’est mesuré.

Garde de licence relue : `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, droits `not_assessed`, `NOASSERTION` personnalisé. Aucun choix de licence ou droit de redistribution n’est affirmé. La mission reste incomplète.

Suite locale utile : définir un résultat JSON actionnable pour erreur de lecture du pipe credential avec les lignes déjà sélectionnées, puis fermer la preuve indépendante de l’exclusion de writers/credential de fusion; le benchmark reste conditionné à un runtime et à des budgets expressément autorisés.
