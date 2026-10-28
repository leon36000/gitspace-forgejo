# RECONCILE-BATCH-OUTPUT-BUDGET-001 — limiter l’amplification des résultats

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. Base inchangée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Lacune et changement

La réconciliation limitait les réponses JSON distinctes mises en cache à 8 MiB et les événements d’un lot à 100, mais les livraisons webhook aux identifiants distincts pouvaient désigner le même commentaire. Chaque résultat copiait alors le corps courant du commentaire. Une réponse courante de 128 KiB répétée 100 fois aurait produit environ 12,5 MiB de corps dans les états de sortie, malgré seulement deux lectures amont (issue parente et commentaire).

`reconcile_inbox_events` sérialise maintenant chaque projection canonique avec le même `reconciled_state_value` que le CLI, cumule ses octets et ajoute une réserve de 2 048 octets par livraison pour l’enveloppe CLI. Au-delà de 8 MiB projetés, la fonction retourne `BatchOutputBudgetExceeded` et ne retourne aucun résultat partiel. Le CLI répond avec `batch_result_budget_exceeded`, `verdict=inconclusive`, `retryable=false`, une limite publiée dans `--capabilities` et l’action de découper le lot sans répéter le lot inchangé. L’assertion de capacité garantit que le plus grand cas d’erreur statique testé (contrat d’événements), avec ID de livraison de 128 caractères et les deux champs d’identité du journal, tient dans la réserve de 2 048 octets.

## TDD et vérification

Le test rouge `repeated_comment_deliveries_fail_closed_when_projected_batch_output_exceeds_budget` ingère 100 livraisons `issue_comment/created` signées par le vrai handler et le journal local, puis utilise une fixture de lecture Forgejo renvoyant un commentaire courant de 128 KiB. Avant la limite, il échoue parce que les 100 états sont retournés; la sortie d’échec ne sérialise pas les gros états. Après correction, le batch échoue fermé et la fixture constate exactement deux lectures amont. Le test de contrat CLI vérifie le verdict, la limite, la non-réessayabilité, l’action et la capacité. Le test d’enveloppe fixe contrôle la réserve d’erreur mentionnée ci-dessus.

Commandes et résultats conservés :

- `cargo test --offline --locked --test webhook_reconcile_pipeline` — 4 réussis, 0 échec.
- `cargo test --offline --locked --bin forgejo-reconcile-local` — 152 réussis, 0 échec avant le dernier ajout d’assertion d’enveloppe; la suite finale ci-dessous inclut cette assertion.
- `cargo test --offline --locked --bin forgejo-reconcile-local tests::unsupported_event_error_points_to_cli_capability_contract -- --exact` — 1 réussi, 0 échec. Une première commande omettait le préfixe `tests::` avec `--exact` et n’a donc exécuté aucun test; sa sortie est conservée dans `envelope-reserve-test.log` et n’est pas comptée comme vérification.
- `cargo test --offline --locked --lib` — 19 réussis, 0 échec, y compris l’unicité parallèle des répertoires temporaires de tests.
- `cargo test --offline --locked --all-targets` — dernier passage complet : 658 réussis, 0 échec, 121 ignorés dans 36 binaires/tests.
- `cargo fmt --all -- --check`, `git diff --check`, `cargo metadata --offline --locked --no-deps --format-version 1` et `python3 scripts/verify_cargo_license_state.py` — réussis.

Le vérificateur append-only confirme que les 18 chemins locaux ont leur empreinte courante, que les index du worktree et du projet pointent sur les dernières empreintes et que les trois préfixes capturés avant le lot sont conservés. `SHA256SUMS` garde aussi les hashes historiques lors de chaque révision du reçu; un `sha256sum -c` naïf examine ces anciennes lignes et signale les versions remplacées. Le vérificateur du dossier compare la dernière entrée par chemin et passe.

Une exécution complète intermédiaire a échoué dans `bounded_replay_reads_a_valid_journal_without_write_permission` pendant `EventJournal::append`, avant l’assertion de permission, avec `InvalidEvent`; le test isolé a réussi immédiatement après. L’inspection a trouvé que le helper test-only nommait les dossiers avec un horodatage et `create_dir_all`, sans allocation exclusive, ce qui autorisait une collision entre tests parallèles. Le helper utilise maintenant un compteur atomique local au processus et `create_dir`; un test parallèle vérifie 32 chemins distincts. L’explication par collision est une inférence : la panne intermédiaire n’a pas conservé le chemin entré en collision. Le passage complet après cette correction est vert; l’échec intermédiaire reste dans `all-targets-final.log`.

## Versions, état Cargo et portée

`cargo 1.95.0 (f2d3ce0bd 2026-03-21)`; `rustc 1.95.0 (59807616e 2026-04-14)`. `cargo metadata` donne `license=null`, `license_file=null`, `publish=[]`; le métadonnée interne indique `no_license_selected`, droits de redistribution `not_assessed`, et `NOASSERTION`. Aucun champ Cargo de licence n’a été choisi ou modifié dans cette tranche.

La source de courant est une fixture locale déterministe : il ne s’agit ni d’un run Forgejo live, ni d’une preuve de charge en production. La limite compte exactement les octets JSON des projections canoniques et ajoute une réserve fixe de 2 048 octets par livraison; elle projette la taille des résultats CLI, elle ne mesure pas l’intégralité du stdout au dernier octet. Le test d’enveloppe vérifie le plus grand cas d’erreur statique conservé, mais pas tous les futurs changements de sérialisation; la réserve devra être maintenue avec les contrats d’erreur. Cette tranche n’intègre aucun LLM/AgentOS, ne mesure ni tokens ni coûts et n’établit aucun gain comparatif.

La preuve reste append-only. Journaux rouge/vert, suite ciblée et complète, Cargo metadata, garde de licence, instantané des préfixes et hashes des sources sont joints au même dossier.
