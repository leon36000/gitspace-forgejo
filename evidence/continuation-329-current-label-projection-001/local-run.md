# CONTINUATION-329 — projection des labels actuels des issues et PR

Date : 27 octobre 2026, 23:48 America/Montreal. Worktree produit : branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; seuls les fichiers cités par ce lot ont été édités. Aucun changement Cargo, commit, push, ni modification Forgejo.

## Lacune et correction

Forgejo émet des événements `issues` avec des actions de labels, et son API d’issue renvoie `labels`. Pourtant la sortie précédente du réconciliateur ne les projetait pas, même après son readback canonique. Les sources Forgejo v17.0.0 lues au commit `0d791421f85ca13a63b665053d81ea765fbbc571` exposent `modules/structs/issue_label.go:Label{ID,Name}` et `modules/structs/issue.go:Issue.Labels`. Le commit est fixé; les identifiants de blobs et empreintes des deux fichiers sont consignés dans `source-hashes.txt`. Aucun code Forgejo n’a été importé.

Le réconciliateur projette maintenant dans `canonical_state.labels` seulement les paires `id`/`name` provenant du readback API courant, dans l’ordre croissant de l’ID. Les labels s’appliquent aux issues, pull requests et snapshots parent de commentaires issue/PR. Le champ absent ou `null` signifie inconnu; un tableau vide est connu vide. La projection limite les réponses à 256 éléments et 255 caractères par nom; structure invalide, ID nul/dupliqué, nom vide/contrôle ou dépassement rend le résultat inconclusif. Les noms sont annotés `untrusted_user_content`; le snapshot de labels dans un webhook n’est jamais utilisé comme canonique.

Le contrat CLI change : single `15`, batch `19`, discovery `forgejo-reconcile-capabilities-v21`. La découverte décrit les ressources, bornes, ordre, fraîcheur, confiance et refus. Les anciennes valeurs de schema sont documentées comme historiques dans les reçus antérieurs et restent inchangées dans ceux-ci.

## Séquence de vérification

Le test test-first `issue_label_projection_uses_current_api_not_stale_webhook_labels` a été exécuté avant l’implémentation. Il a échoué sur l’assertion de sortie attendue : valeur observée `null`, valeur attendue l’objet des labels issus de l’API; code de sortie 101. Après modification du réconciliateur :

- `webhook_reconcile_contract` : 84 réussis, 0 échec.
- `forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities` : 1 réussi.
- `actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture`, dans `bwrap --unshare-net` : 1 réussi, 167 filtrés. Le vrai processus receiver accepte les livraisons synthétiques; le vrai CLI fait un GET local et retourne les labels API triés pour les deux lignes du batch. Le serveur HTTP est une fixture sur loopback, pas Forgejo.
- Tests de compatibilité schema : `reconcile_config_contract` 4, `reconcile_journal_lock_timeout` 3, `webhook_quarantine_cli` 5, tous réussis au rerun. Le premier run a trouvé un test de refus quarantaine encore figé à v14; l’assertion est passée à v15, sans changer le comportement testé.
- Trois validateurs Python qui consomment les réponses CLI avaient également des contrôles v14/v18. Leur premier run a échoué sur l’enveloppe avant d’atteindre les assertions métier; cinq validateurs ont été alignés sur v15/v19. Les suites `test_forgejo_emitted_webhook_e2e.py` (131), `test_forgejo_live_org_team_reviewer_readback.py` (20) et `test_forgejo_webhook_native_replay_e2e.py` (30) passent alors 181/181. Ces scripts sont validateurs unitaires/synthétiques; aucun serveur GitLab/Forgejo n’a été contacté.
- `cargo test --offline --locked --all-targets --no-run` : toutes les cibles compilent; aucune assertion générale de réussite suite à cette seule compilation.
- `cargo fmt --all -- --check`, `git diff --check`, garde de métadonnées Cargo et analyse AST des trois tests Python dont les fixtures de versions ont été ajustées : réussis.

Les journaux verts Python et Rust sont enregistrés; `schema-compatibility-tests.log` conserve le premier échec Rust sur l’ancienne version. Le premier lancement Python a affiché des échecs d’enveloppe, mais son output n’a pas été redirigé vers un fichier; le rapport résume ce constat sans le présenter comme un journal natif. Le résultat rouge du TDD a aussi été observé dans la sortie de l’outil avant l’implémentation mais non enregistré dans un fichier; la différence d’assertion est rapportée comme observation de session.

## Couverture et limites

La route API est exercée par le vrai CLI Rust, sous namespace réseau isolée, mais le corps et la réponse HTTP sont synthétiques. Ce lot ne fait ni readback d’une instance Forgejo live, ni ingestion native d’un vrai événement émis, ni travail d’agent/LLM, ni PR/fusion. Il ne démontre pas qu’un agent consomme ces labels utilement, ni l’effet sur exigences satisfaites, régressions, interventions, durée ou tokens. Les labels sont des données contextuelles non fiables, non des exigences ou approbations.

L’interface AgentOS, le transfert GitLab PC4, la licence GitSpace, l’exclusion globale des writers Forgejo et la comparaison appariée restent inchangés et ouverts. Ce lot ne reprend aucune communication distante. Le contrôle Cargo reste sans choix de licence et sans droit de redistribution affirmé.
