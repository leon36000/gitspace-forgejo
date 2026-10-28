# CONTINUATION-267 — conserver les sources sélectionnées lors d’un refus de budget batch

Date : 27 octobre 2026. Worktree local fortement modifié, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, nettoyage, commit, publication, suppression ou changement de production. Les manifestes locaux, du worktree et du projet sont append-only; leur préfixe historique est archivé dans `manifest-prefix-before.json`.

## Changement observé

`forgejo-reconcile-local` expose maintenant `selected_event_sources` uniquement lorsque la réconciliation batch échoue par `BatchOutputBudgetExceeded`. Chaque objet lie l’ID sélectionné à la ligne locale correspondante, dans l’ordre demandé, avec `journal_source.sequence`, `journal_source.event_sha256` et l’observation de l’attestation receiver. La provenance annonce explicitement qu’elle ne prouve pas l’authenticité du writer local. Le résultat reste `inconclusive`: aucune ligne `results`, aucun état canonique ni aucun fragment des commentaires volumineux n’est rendu. Les erreurs d’un autre type ne reçoivent pas ce nouveau champ. Single reste en schéma v13; batch passe à v15; capacités passent à v13.

Le test de processus `actual_reconcile_cli_process_refuses_repeated_comment_batch_output_budget` a créé 100 livraisons par le receiver réel, arrêté le receiver, puis lancé le binaire réel sous Bubblewrap contre un fixture HTTP local. Deux lectures GET (parent et commentaire) retournent un commentaire de fixture de 128 KiB; la projection dépasse le budget de sortie. Le processus retourne un refus actionnable invitant à fractionner le batch, avec `results` absent, 100 entrées sources dans le même ordre, séquence/hash égaux au journal relu, attestation `missing`/`not_verified`, signature omise, corps partiel absent, stdout inférieur à 64 KiB et journal inchangé. Le test ciblé passe 1/1. Le fixture n’envoie pas de credential Authorization.

## Vérifications

La commande complète `cargo test --locked --offline --all-targets` s’est terminée avec le code 0 : 686 tests réussis, zéro échec et 144 ignorés sur 37 blocs de résultats. `cargo check --locked --offline --all-targets`, `cargo fmt --all -- --check`, `git diff --check`, la garde Cargo et ses 12 tests passent. Les trois modules Python de consommateurs batch passent 190 tests; `py_compile` des cinq scripts et trois modules concernés passe.

Un journal focalisé conservé rapporte 159 tests du binaire et quatre groupes de contrats (5/2 ignorés, 4, 3 et 5 réussis); le run `--all-targets` ci-dessus est la vérification finale complète. Le test Bubblewrap ciblé passe 1/1 (157 filtrés). Logs, commandes et versions sont dans ce dossier. Les échecs de diagnostic sont conservés : `process-budget-red.log` enregistre une erreur de type dans une première assertion de test; `process-budget-red-functional.log` échoue ensuite comme attendu sur batch v14 au lieu de v15. La suite focalisée a relevé un contrat de capacités obsolète v12 et une chaîne de contrat absente dans une assertion de configuration, corrigés avant le run vert. `process-budget-green.log` a échoué sur l’ancienne borne stdout de 16 KiB; la borne de test a été portée à 64 KiB, et le test impose encore strictement cette limite sans contenu partiel. `process-budget-green2.log` contient le succès final.

## Portée et limites

C’est un test local du CLI et du receiver réels dans Bubblewrap, avec un fixture HTTP scripté; ce n’est pas un test contre Forgejo live. Il vérifie le comportement de sérialisation et l’association sélection-ID/ligne de journal pour ce cas de dépassement; il ne démontre pas que le journal n’a pas été forgé par un writer de même UID, ni la vérité d’un événement amont. La signature Ed25519 facultative reste une attestation de provenance du receiver, non une preuve de correction. Le HMAC source n’est pas revérifié au replay. Aucun LLM, AgentOS, gain en qualité/temps/tokens/coût ou comparaison CI conventionnelle n’est démontré.

La garde Cargo retourne toujours `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed` et le marqueur personnalisé `NOASSERTION`; il ne s’agit ni d’un choix de licence ni d’une validation juridique. Licence GitSpace et droits de redistribution demeurent indéterminés.

La mission reste incomplète. Les besoins encore ouverts comprennent notamment l’exclusion indépendante des writers et du credential de fusion, les refus composés non couverts et la comparaison appariée avec Forgejo + agent + CI conventionnels lorsque runtime et budgets autorisés sont disponibles.
