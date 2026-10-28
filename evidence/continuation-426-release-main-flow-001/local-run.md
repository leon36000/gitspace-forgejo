# CONTINUATION-426 — Release câblé au runner principal

Date d’exécution : 2026-10-28 UTC. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt comporte déjà des modifications locales et 15 137 entrées dans `git status` au relevé initial; les trois fichiers de ce reçu portent le compte courant à 15 140. Aucune remise à zéro, suppression, staging ou commit n’a été fait.

## Changement vérifiable

Le runner `scripts/run_forgejo_live_webhook_reconcile_emitted.py` proposait le contrat de release mais n’appelait le helper de réconciliation que depuis ses tests. Le mode opt-in `--release-webhook` le relie maintenant à `run()` : le hook existant ajoute le sélecteur `release` seulement pour cette option, puis une publication jetable est créée avant les événements PR. Le runner attend le nombre exact de lignes attendu, prend le dernier enregistrement, valide son dépôt, action `published`, ID, delivery unique et séquence égale à sa position dans le journal. Il appelle ensuite le CLI `forgejo-reconcile-local` avec le credential lecteur déjà limité au dépôt et la clé d’attestation connue. Le résultat est accepté seulement si la lecture API courante lie le delivery, l’attestation, l’ID de ressource et le tag publié; l’action et le tag reçus restent des données non fiables. Le rapport d’un run natif contiendra `release-event-reconciliation.json`. Sans l’option, le hook et les compteurs d’événements par défaut restent ceux d’avant.

Le test d’orchestration injecte une API et un exécutable de réconciliation fixture; il vérifie la création HTTP, l’événement journalisé, le passage effectif au runner `run_reconcile`, l’argument credential lecteur, l’attestation et le readback lié au tag. Un second test lance le vrai helper `run_reconcile` face à un processus enfant fixture et vérifie que le credential lecteur est lisible seulement depuis le FD privé, sans présence dans argv ou la sortie structurée. Les refus vérifient une action différente et une séquence non contiguë avant tout appel au CLI. Les tests de composition vérifient aussi les bindings de delivery et les projections de readback. Aucun de ces processus n’est un serveur Forgejo.

## Vérifications

- TDD rouge : le nouveau test de sélection du hook et les tests du nouveau parcours échouaient avant l’implémentation, car leurs points d’entrée n’existaient pas.
- `PYTHONPATH=tests python3 -m unittest test_forgejo_emitted_webhook_e2e.ReleaseWebhookBindingTests test_forgejo_emitted_webhook_e2e.ReleaseReconciliationCompositionTests test_forgejo_emitted_webhook_e2e.ReleaseWebhookMainFlowTests` — **13/13 réussis**.
- `PYTHONPATH=tests python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` — **184/184 réussis**.
- `python3 scripts/test_cargo_license_state.py` — **12/12 réussis**.
- `python3 scripts/verify_cargo_license_state.py` — `metadata_policy_verified`; metadata observée : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, marqueur d’export `NOASSERTION`.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py && git diff --check` — réussi.
- `python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --help | rg -- '--release-webhook|Usage:'` — réussi, l’option est exposée.

Les versions et les commandes reproductibles sont consignées dans `environment.txt` et `commands.txt`.

## Limites observées

`forgejo` et `go` sont absents du `PATH` de cet environnement; le mode natif n’a donc pas été lancé. Cette preuve couvre le flux principal par code et par tests de processus/API simulés, pas un événement émis par Forgejo. Elle ne prouve pas une PR, une fusion ou le comportement de toutes les versions. Aucun appel de modèle, AgentOS ou Forge Code local n’a été identifié dans `src`, `Cargo.toml`, `README.md` et `docs/architecture.md`; pas de nouvel orchestrateur/RAG/mémoire ajouté. Aucun test Rust ni `cargo build` n’a été nécessaire à cette modification Python. `/` n’a que 415 MB disponibles. Le runner actuel épingle son état et le staging de compilation sous `/tmp`; l’espace libre de `/dev/shm` n’est pas automatiquement utilisé par ce chemin. La taille exacte du sous-ensemble Cargo copié n’a pas été mesurée, donc le run natif n’a pas été lancé sous ce risque de remplissage disque.

La politique de licence Cargo est conservée telle quelle : aucune licence n’est choisie et les droits de redistribution ne sont pas évalués. Cela évite d’inventer une décision juridique; ce n’est pas une résolution des droits du produit. GitSpace reste non copié. Aucun GitLab n’a été consulté ou modifié dans ce lot. Le checkpoint antérieur rapporte un transfert partiel de 134 fichiers; il ne représente pas le transfert complet des preuves et archives. Les valeurs d’accès signalées comme exposées dans le courriel ne sont ni relues ni réutilisées.

## Suite

Quand un binaire Forgejo v17 jetable sera disponible dans un environnement local, exécuter le nouveau mode sur une instance temporaire, lire la livraison native et son état courant, vérifier le reçu et conserver les artefacts sans inclure secrets ou notes de release. Ensuite poursuivre les critères encore ouverts du checkpoint, en distinguant toujours fixture, exécution locale intégrée, modèle réel et comparaison contrôlée. La mission n’est pas terminée.
