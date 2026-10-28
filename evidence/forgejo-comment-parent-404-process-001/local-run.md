# FORGEJO-COMMENT-PARENT-404-PROCESS-001 — ne pas inférer l’absence du commentaire si son issue est inaccessible

Date : 26 octobre 2026, 21:23 UTC. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt déjà fortement modifié. Aucun reset, commit, publication, changement de production ou code Forgejo copié.

`tests/webhook_receiver_process_e2e.rs` et `scripts/run_webhook_reconcile_cli_e2e.py` sont actuellement non suivis (`??`) dans le worktree; aucune mise en index ou commit n’a été faite. Leur existence et leurs octets utilisés sont hashés dans ce reçu; `source-whitespace-check.json` vérifie qu’ils se terminent par une nouvelle ligne et ne contiennent pas d’espaces de fin.

## Question

Pour un événement `issue_comment/deleted`, un 404 sur le commentaire prouve une suppression uniquement après lecture réussie de l’issue parente. Le contrat Rust couvrait cette règle, mais il manquait un parcours du vrai receiver et du vrai CLI quand le parent lui-même est illisible. Le nouveau test de processus couvre cette branche de refus, sans prétendre qu’une fixture représente les permissions de Forgejo.

## Parcours

```sh
python3 scripts/run_webhook_reconcile_cli_e2e.py \
  --scenario actual_reconcile_cli_process_keeps_deleted_comment_inconclusive_when_parent_issue_is_unreadable
```

Le runner construit les binaires hors ligne et lance le test dans Bubblewrap 0.9.0 avec namespaces réseau et PID isolés; la fixture API loopback est autorisée pour ce scénario. Le receiver Rust accepte une livraison signée de test, l’inbox est ensuite lue par `forgejo-reconcile-local`, puis l’API simulée renvoie un 404 pour `/repos/forge-admin/agent-forge-core/issues/24`. Le commentaire est #916.

## Résultat

Le CLI termine avec code 1, `verdict=inconclusive`, `error.code=resource_not_found`, `retryable=false`, `upstream_read_attempts=1` et lie précisément l’issue #24 et le commentaire #916. Il ne fait qu’un GET de l’issue parente, n’appelle pas la route du commentaire, ne fournit pas de `canonical_state`, n’envoie pas d’en-tête `Authorization`, n’expose ni le corps HTTP 404 ni le texte du commentaire, et laisse l’inbox identique octet pour octet. Le résultat ciblé final passe 1/1 avec 109 tests filtrés.

## Vérifications

- `cargo test --offline --locked --test webhook_reconcile_contract` : 53/53.
- `python3 -m unittest -v scripts.test_cargo_license_state` : 9/9; `python3 scripts/verify_cargo_license_state.py` : `status=verified`.
- `python3 -m unittest -v tests.test_forgejo_emitted_webhook_e2e` : 112/112.
- `py_compile` sur le runner de réconciliation, le runner Forgejo live et leurs tests : code 0.
- `cargo fmt --all -- --check` : passe après formatage ciblé du seul fichier Rust modifié.
- `git diff --check` : passe.
- Le contrôle whitespace des deux sources non suivies : passe; `git diff --check` à lui seul n’inspecte pas ces fichiers non suivis.
- Cargo metadata hors ligne, verrouillée et sans dépendances : `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; métadonnées projet `no_license_selected`, `redistribution_rights=not_assessed`, marqueur `NOASSERTION`. Aucun choix de licence ni conclusion juridique.

Le premier `cargo fmt --all -- --check` a détecté un format incorrect dans le nouveau test. Cet échec est gardé dans `rustfmt-check.log`; `rustfmt --edition 2024 tests/webhook_receiver_process_e2e.rs`, le contrôle global final et le test processus final passent. Les logs complets, le readback Cargo, les versions et les empreintes avant/après formatage sont dans ce dossier.

## Portée et limites

Le receiver et le CLI compilés sont réels, mais l’API Forgejo est une fixture loopback et l’événement HMAC est synthétique; aucun processus Forgejo live n’est utilisé par cette preuve. Elle confirme l’ordre des deux lectures et le refus prudent du CLI, pas le comportement d’une permission live ni tous les codes HTTP. Le scénario est déterministe, un test et un dépôt configuré. Aucun AgentOS, LLM, RAG, mémoire, service externe, secret externe ou coût modèle n’est utilisé. C’est une garantie testée au niveau processus, pas un changement du code de réconciliation déjà présent dans `src/`.

Les sources testées, leurs commits/fichiers, versions d’outils et résultats sont enregistrés dans `source-hashes.txt`, `toolchain.json`, `command-results.json` et `final-verification.json`. L’intégrité append-only locale, du worktree et du projet est ajoutée après finalisation du checkpoint et de la matrice.
