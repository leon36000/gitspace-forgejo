# CONTINUATION-144 — diagnostic du `Retry-After` invalide ou ambigu

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste fortement modifié; le fichier `Cargo.toml` y était déjà modifié avant cette tranche. Aucun reset, commit, publication ou changement de production.

## Changement et test first

CONTINUATION-143 avait établi que le CLI n’effectuait pas de retry quand une réponse `429` contenait plusieurs `Retry-After`, mais l’erreur structurée ne disait pas pourquoi `retry_after_ms` manquait. Le test processus existant a d’abord été renforcé avec deux assertions sur le motif et l’action. Le run rouge a terminé en échec attendu : `details.retry_after_disposition` était `null`, au lieu de `invalid_or_ambiguous` (0 passé, 1 échec, 119 filtrés). L’extrait observé est conservé dans [red-run-observed.txt](red-run-observed.txt).

Le code transmet maintenant ce cas sous `ApiReadError::HttpStatusWithInvalidRetryAfter { status }`. Les sérialisateurs de réconciliation et de préflight conservent `http_status`, ajoutent `retry_after_disposition=invalid_or_ambiguous` et conseillent de ne pas utiliser cette valeur pour un retry automatique; le test processus confirme la sortie de réconciliation. Le statut `retryable` reste une classification de transience; l’appelant doit appliquer son budget et obtenir une lecture fraîche. La valeur de l’en-tête et le corps amont ne sont jamais recopiés. L’énumération Rust publique a reçu une variante : une consommation externe avec `match` exhaustif pourrait nécessiter une adaptation; aucun consommateur externe ni publication de ce crate n’est établi ici.

## Vérification finale

Le vrai receiver accepte une livraison `issues` HMAC synthétique en HTTP 202; le vrai `forgejo-reconcile-local` lit l’inbox et interroge une fixture HTTP loopback qui retourne `429` avec `Retry-After: 0` et `retry-after: 0`. Le scénario exact passe **1/1** après formatage avec Bubblewrap 0.9.0, `--unshare-net --unshare-pid`, compilation Cargo offline et 119 tests filtrés. Il vérifie `inconclusive/upstream_http_status`, le motif structuré et l’action, `retryable=true`, l’absence de `retry_after_ms` et de `canonical_state`, une seule requête observée durant 300 ms, aucun `Authorization`, aucune fuite du corps et une inbox inchangée octet pour octet. Sortie : [focused-run-post-format.log](focused-run-post-format.log).

`cargo test --offline --locked --bin forgejo-reconcile-local` passe **151/151**. `cargo test --offline --locked --all-targets --quiet` passe sur 36 cibles, **629 réussis, 0 échec, 111 ignorés**; le scénario de processus isolé est ignoré par Cargo normal et a donc été lancé séparément. Reçus : [reconcile-cli-tests.log](reconcile-cli-tests.log) et [all-targets.log](all-targets.log). `cargo fmt --all -- --check` et `git diff --check` finaux passent; le premier format-check après ajout d’un commentaire doc avait signalé seulement un formatage d’énumération, conservé dans `rustfmt.log`, puis corrigé selon l’édition 2024 avant la vérification finale.

`cargo metadata --locked --offline --no-deps` et le garde local confirment `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, `NOASSERTION`. Aucun manifeste de dépendances n’a été modifié par cette tranche. Sous `src/` et `tests/`, la seule implémentation de `AgentRuntimeV1` reste `FixtureRuntime`; aucun LLM, AgentOS, service externe, secret réel ou coût modèle n’a été utilisé.

## Limites

Un scénario, deux valeurs égales à zéro, une fixture synthétique et une fenêtre de 300 ms; aucun Forgejo live, toutes permutations/valeurs, appelant externe, politique de retry distribuée, LLM, AgentOS, benchmark comparatif ou gain n’est démontré. Le test observe l’absence de deuxième requête pendant cette fenêtre, pas une garantie temporelle générale. Le résultat n’autorise pas une fusion. Le projet reste incomplet.

Commandes, versions, résultat résumé et empreintes : [commands.txt](commands.txt), [tool-versions.txt](tool-versions.txt), [revision.txt](revision.txt), [verification-summary.json](verification-summary.json), [source-sha256.txt](source-sha256.txt), [cargo-metadata.json](cargo-metadata.json), [license-guard.log](license-guard.log) et les manifestes append-only locaux/globaux.
