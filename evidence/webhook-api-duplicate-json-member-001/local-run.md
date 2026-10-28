# WEBHOOK-API-DUPLICATE-JSON-MEMBER-001 — refuser les réponses Forgejo à clés ambiguës

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, fortement modifié et sans commit dédié.

## Défaillance reproduite et correction

Le parseur central de `LocalLoopbackForgejoApi::request_api_once` décodait les corps HTTP 200 avec `serde_json::from_slice::<Value>`. Le test rouge lui a servi `{"number":11,"number":12}`; `read_current` a renvoyé `Ok({"number":12})` au lieu de `ApiReadError::InvalidResponse`. Le log `test-red-effective.log` contient cette assertion et le code de sortie 101. Une première commande avec `--exact` n’avait fait tourner aucun test (0 exécuté); elle est conservée sous `test-filter-mismatch.log` et n’est pas considérée comme une preuve.

Le nouveau module commun `src/strict_json.rs::parse_value_rejecting_duplicate_members` reconstruit les valeurs JSON en rejetant les clés répétées à toute profondeur et compare les chaînes après décodage des échappements. L’ingestion webhook HMAC l’utilise après vérification de signature; le parseur central des réponses API 200 l’utilise avant de rendre un `Value`. Les erreurs webhook deviennent `InvalidJson`, les réponses API deviennent `ApiReadError::InvalidResponse`. `gate_api_json`, les GET de réconciliation et le readback de fusion partagent le point de décodage `request_api_once`. Un second test appelle directement `merged_readback` avec `merged:true` et `merged:false` dupliqués; il reçoit `InvalidResponse` et le fixture ne sert qu’une seule requête. Il ne tente pas de fusionner une PR.

## Résultats

- Le test API ciblé passe 1/1, exige `InvalidResponse`, vérifie une seule requête et aucun retry.
- Les deux tests d’ingestion webhook passent 2/2; le vrai sous-processus `forgejo-webhook-local` refuse les deux formes HTTP 400 et laisse le journal inchangé : 1/1.
- Le test de readback post-fusion avec clé critique dupliquée passe 1/1. Le premier essai avait une assertion erronée sur `upstream_read_attempts`, qui ne compte pas `gate_api_json`; la vérification `InvalidResponse` était déjà passée. Le test corrigé vérifie le nombre de requêtes réellement servies par le fixture. Les deux sorties sont gardées.
- `cargo test --offline --locked --all-targets -- --test-threads=1` passe 490 tests, 0 échec et 55 ignorés sur 34 groupes. `all-targets-postmerge-refusal.log` contient le test API, le refus du readback et le test du processus receiver.
- `cargo fmt --all -- --check`, `rustfmt --edition 2024 --check` sur les fichiers touchés et `git diff --check` passent.
- Clippy bibliothèque passe avec les allowances historiques `too_many_arguments` et `collapsible_if`. Clippy strict all-targets reste en échec sur les avertissements existants dans d’autres modules; le log est conservé.

Le rouge TDD est reproductible; les commandes exactes et sorties sont dans ce dossier. Le manifeste local couvre 38 artefacts et passe 38/38 (`SHA256SUMS`). Cargo n’a pas changé; le readback de ce lot confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`. `license-guard.log` rapporte `verified` et `license-tests.log` passe les sept tests du garde; aucun choix de licence.

## Portée et limites

Les tests API et de readback utilisent un fixture HTTP loopback et des données synthétiques; ils ne démarrent pas Forgejo et ne démontrent pas son comportement en production. Le test de fusion appelle `merged_readback` directement; il ne tente pas un merge ni ne couvre tout le gate de fusion. Les 55 tests ignorés ne sont pas couverts par le run générique. Aucun LLM, AgentOS, Forge Code, RAG ou mémoire n’a été invoqué; l’implémentation n’ajoute ni exécuteur ni fournisseur. Aucun service externe, secret utilisateur, coût, publication ou environnement de production n’a été utilisé.

## Environnement

Rust `1.95.0`, Cargo `1.95.0`, Python `3.12.3`, Linux aarch64. Voir `commands.txt`, `tool-versions.txt` et `source-sha256.txt`. Le parseur webhook avait été implémenté dans `src/webhook.rs` au moment du premier lot; cette tranche le factorise dans `src/strict_json.rs` et l’emploie également pour les réponses de l’adaptateur.
