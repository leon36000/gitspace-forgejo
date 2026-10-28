# CONTINUATION-021-ACTIONABLE-ERROR-PROPAGATION

Date : 26 octobre 2026. Produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà sale et fortement modifié; aucune modification préexistante n'a été supprimée ni commise.

## Résultat

Ce lot ferme deux refus qui n'étaient pas assez actionnables.

### Sortie superviseur vers le local-test-adapter

Le statut 76 du superviseur indique qu'au moins une demande de kill de scope n'a pas été acceptée. Le fixture `mission_012_test_agent_adapter.py` remontait seulement « MCP supervisor exited with status 76 ». Il produit maintenant une erreur qui nomme l'échec de demande de cleanup et précise que le résultat du candidat est inconclusif.

Le test `tests/test_mission_012_test_agent_adapter.py::test_nonzero_supervisor_cleanup_status_becomes_adapter_error_after_verification` fournit des réponses MCP contrôlées pour mission context, baseline rouge, lectures, écriture et vérification verte, puis fait retourner 76 par le faux `MCPClient.close`. `main` sort code 1 avec `type=error`; aucun statut `success` ou `interrupted` n'est émis. La sortie rouge est conservée dans [red-adapter-status.log](red-adapter-status.log).

### Champs `ref` et `ref_type` des branches Forgejo

`validate_event_payload_kind` refusait déjà ces payloads avant journalisation, mais un `ref` invalide et un `ref_type` absent donnaient `details.field_path=null`. Les cas `create` et `delete` renvoient désormais `InvalidEventPayloadField("ref")` quand le champ ref est absent, non textuel ou invalide, et `InvalidEventPayloadField("ref_type")` quand le champ est absent ou non textuel. Un `ref_type` texte différent de `branch` reste hors périmètre et refusé.

Les tests de contrat du handler couvrent les branches mal formées, champs absents et mauvais types. Un test du vrai processus receiver envoie un `create` signé par secret fixture sans `ref_type`; le receiver répond HTTP 400, `event_payload_invalid`, `retryable=false`, `payload.ref_type`. Le fichier inbox reste octet pour octet identique et son rejeu est vide. Les deux contre-exemples rouges sont conservés dans [red-handler-ref.log](red-handler-ref.log) et [red-receiver-ref-type.log](red-receiver-ref-type.log).

Le wire source primaire est Forgejo v17.0.0 au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`, fichier `modules/structs/hook.go`, blob `9c7f1deed1c38eeb435ab976bb8429c5458c7f2b`: `CreatePayload` et `DeletePayload` déclarent `Ref`/`RefType` comme chaînes JSON; `CreatePayload` contient aussi `Sha`. Le HEAD du scratch local est ce même objet, et `git diff --quiet <commit> -- modules/structs/hook.go` retourne 0. Ce relevé porte sur ces symboles seulement; aucun code Forgejo n'est copié.

## Vérifications

- `python3 tests/test_mission_012_test_agent_adapter.py` : 1/1 réussi.
- `python3 tests/test_mission_012_sandboxed_test_adapter.py` : 5/5 réussis.
- Test ciblé du handler : 1/1 réussi; suite dans le run global.
- Test ciblé du vrai receiver : 1/1 réussi; suite dans le run global.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 551 réussis, 0 échec, 76 ignorés, 34 groupes. Les ignorés ne sont pas exécutés. Journal complet : [rust-all-targets.log](rust-all-targets.log).
- Python `py_compile`, `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` réussissent; les sorties sont dans ce dossier.
- `cargo metadata --offline --locked --no-deps --format-version 1` confirme pour `agent-forge-core`: `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, droits `not_assessed`, `NOASSERTION`. La licence reste non choisie.
- `AgentRuntimeV1` est seulement un trait dans `src/agent_runtime.rs`; la seule implémentation trouvée dans `src/` et `tests/` est `FixtureRuntime` dans les tests. Le `PATH` ne fournit aucun des CLI locaux vérifiés : Ollama, llama.cpp, LM Studio, OpenCode, Aider ou Goose. Aucun modèle n'a été appelé.

Versions, commandes, résultats, inventaire du runtime, recherche de CLI, contrat amont et SHA-256 sont voisins. Les manifestes du produit et du parent sont append-only; le script de mise à jour vérifie chaque empreinte courante sans réécrire leurs préfixes.

## Limites et prochaine étape

Le flux de statut 76 est testé dans le consommateur fixture avec un faux client MCP; cette tranche ne le force pas à travers un vrai superviseur, une vraie session MCP puis Bubblewrap. Le protocole MCP ne retourne pas de réponse JSON-RPC après la fermeture forcée du serveur. Le webhook est signé avec un secret de test et atteint le vrai receiver local, mais n'est pas émis par une instance Forgejo live. Ce lot ne prouve pas un parcours LLM/AgentOS ni une amélioration comparative.

Aucun coût, accès, secret utilisateur, service externe, production ou publication. Prochaine tranche locale : injecter le statut 76 dans la session MCP/sandbox complète et vérifier que l'échec reste visible à l'adaptateur hôte; puis choisir un cas de réconciliation/adapter distinct non déjà couvert. La limite Forgejo sur l'autorité serveur des writers et la récupération LFS reste hors de ce lot. Le projet demeure incomplet.
