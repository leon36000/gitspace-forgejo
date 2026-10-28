# WEBHOOK-CANONICAL-URL-ORIGIN-PROCESS-001 — refus au niveau processus

Date : 25 octobre 2026. Révision de travail observée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. La révision ne contient pas les fichiers non suivis du test et de son runner; les empreintes de ces fichiers et des sources consultées sont dans `source-sha256.txt`.

## Parcours vérifié

Le scénario `actual_reconcile_cli_process_rejects_foreign_canonical_html_url` traverse le vrai binaire `forgejo-webhook-local` démarré comme sous-processus, puis le vrai `forgejo-reconcile-local`. Le test crée un secret HMAC synthétique, envoie une issue #41 signée et valide l’acceptation HTTP `202`, l’ID de livraison et l’écriture durable. Il arrête le récepteur et conserve une copie octet pour octet de l’inbox.

Une fixture HTTP privée renvoie ensuite `200` et un objet qui a un numéro, un état et un horodatage valides, mais un `html_url` sous `https://attacker.invalid/forge-admin/agent-forge-core/issues/41`. Le CLI demande une seule fois `/api/v1/repos/forge-admin/agent-forge-core/issues/41`, sans en-tête Authorization. Il échoue avec le JSON schema 3, verdict `inconclusive`, code `resource_mismatch`, `retryable=false`, ressource `issue/41` et sans `canonical_state`. L’inbox reste identique.

Le test passe par Bubblewrap avec `--unshare-net --unshare-pid`; la fixture n’écoute que sur loopback. Le scénario final après formatage passe `1/1`, code `0`; les 40 autres tests processus sont filtrés. Il n’y a pas de connexion à une instance Forgejo réelle.

## Vérifications et provenance

La règle de validation de l’origine canonique existait déjà dans `src/webhook/reconcile.rs`; ce lot n’en modifie pas l’implémentation. Il ajoute un test processus et l’enregistre dans `scripts/run_webhook_reconcile_cli_e2e.py`. `cargo fmt --all -- --check`, le parse AST du runner, `git diff --check` et `scripts/verify_cargo_license_state.py` terminent tous avec code `0`. L’état Cargo est conservé dans `cargo-license-state.json`: `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`.

Versions relevées : Rust/Cargo `1.95.0`, Python `3.12.3`, Bubblewrap `0.9.0`; détail dans `tool-versions.txt`. Les sources exactes sont liées par SHA-256 dans `source-sha256.txt`; le manifeste local couvre les journaux, commandes, versions, rapport et readback de licence. Les commandes reproductibles sont dans `commands.txt`.

## Portée

Ce résultat prouve un refus composé au niveau receiver → journal durable → CLI → fixture HTTP, pas le comportement contre un serveur Forgejo live, ni une PR, une fusion, un agent LLM ou l’exécution d’un code candidat. Le `html_url` étranger est une entrée de fixture. Aucune modification de production ou de Forgejo n’est apportée; aucun service externe ni modèle n’est appelé. Le parcours LLM et la comparaison Forgejo + agent + CI contrôlée restent non démontrés; la mission globale demeure incomplète.
