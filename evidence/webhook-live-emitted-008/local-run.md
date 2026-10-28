# WEBHOOK-LIVE-008 — événements PR émis par Forgejo (24 octobre 2026)

## Reproduction

Depuis la racine du worktree `local/WEBHOOK-005-canonical-links`, parent `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` :

```sh
python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py
python3 scripts/verify_cargo_license_state.py
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-008-20261024-01 \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-008"
(cd evidence/webhook-live-emitted-008 && sha256sum -c SHA256SUMS)
```

Le runner construit les binaires Rust via Bubblewrap 0.9.0, avec namespaces réseau et PID, environnement nettoyé, sources/registre/sysroot en lecture seule et Cargo `--offline --locked`. Le reçu de build, toolchain, hashes source et binaires est dans [`provenance.json`](provenance.json). Les huit tests Python ciblés passent; `py_compile`, le garde de licence et les sommes passent aussi. Le champ de licence Cargo demeure `null`, sans décision de redistribution (`not_assessed`).

## Résultat observé

Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, a créé un dépôt privé jetable sur `127.0.0.1:3130`. Le hook a émis quatre événements réels vers le receiver Rust : issue #1 `opened`, issue #1 `closed`, pull request #2 `opened`, pull request #2 `closed`. Les quatre tâches internes Forgejo rapportent livraison réussie, statut HTTP 202; chaque UUID correspond au `delivery_id` de même rang dans le journal.

La PR contient un vrai commit ajoutant `webhook-e2e.txt` sur `agent/webhook-e2e`; la tête observée est `82d316bc4dbd9f3568f6e50ea82658bd2109e4bf`, la base `8c61222bd235bfb2e8f1dcf55b876b7a975c89a6`. Forgejo API confirme la PR fermée et non fusionnée. Les notifications d’ouverture et de fermeture, réconciliées séparément par `forgejo-reconcile-local` avec un lecteur non administrateur sans droit push, retournent toutes deux l’état canonique actuel `closed`, `merged=false` et ces mêmes SHAs. Les deux événements d’issue retournent également l’état courant `closed`. Le credential invalide reste `inconclusive/upstream_unauthorized`, sans projection canonique.

L’API confirme la désactivation du hook. Forgejo et le receiver sortent avec code 0; les ports testés ne sont plus en écoute. Le répertoire privé `/tmp/forgejo-live-webhook-emitted-008-20261024-01` reste conservé en mode `0700`; ses secrets ne sont pas copiés dans les artefacts. Le dossier de preuves est lui aussi en mode `0700`; chaque fichier de reçu est en mode privé et couvert par [`SHA256SUMS`](SHA256SUMS).

## Portée et limites

Cette exécution démontre l’émission Forgejo réelle des deux types d’événements, le transport HMAC, le journal, le binding des UUID aux tâches et la relecture de l’état courant. Elle ne vérifie ni n’exécute le code de la PR et ne la fusionne pas. Le parcours antérieur MISSION-012 exerce séparément une correction rouge/verte, un vérificateur indépendant et une fusion contrôlée; ces runs ne forment pas une seule transaction E2E. Ce run utilise la table SQLite interne `hook_task` de Forgejo v17 pour lire les tâches. Forgejo, receiver et CLI partagent l’UID hôte; aucune exécution de code candidat hostile n’a lieu. Le binaire est identifié par hash, mais son origine de build depuis le tag officiel Forgejo n’est pas établie par ce reçu. Aucun LLM, AgentOS, Forge Code, RAG ou mémoire n’a été utilisé ici; aucune comparaison de succès, interventions, temps, tokens ou coût n’est établie.

## Essai Claude demandé

Un appel local Claude Code séparé avec `claude-opus-5`, boucle Ralph limitée à six itérations et plafond CLI configuré à 1 USD s’est arrêté dès le premier tour avec `error_max_budget_usd`. Le coût estimé rapporté est `1.0145015 USD`; un sous-agent Sonnet avait terminé une tâche en lecture seule. Aucun changement de source n’a été produit et aucune itération Ralph supplémentaire n’est démontrée. Aucun nouvel appel payant n’a été lancé après ce dépassement.
