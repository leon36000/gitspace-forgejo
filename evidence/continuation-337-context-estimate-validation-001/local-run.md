# CONTINUATION-337 — cohérence de l’estimation de contexte

Date : 28 octobre 2026. Dépôt : worktree `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié. Ce lot touche uniquement le contrat local Rust de contexte et sa documentation; il n’exécute ni modèle ni instance Forgejo.

## Défaut et correction

`AgentExecutionRequestV1::validate` vérifiait `ContextManifest.estimated_tokens <= token_budget` mais ne comparait pas le nombre annoncé au texte `ContextPacketV1.text`. Le test rouge construit un texte de 200 caractères `é` (400 octets UTF‑8), annonce 99 tokens pour un budget de 100, et attend le refus. Avant la garde, `validate()` retournait `Ok(())` et `unwrap_err()` provoquait l’échec du test (code 101, sortie dans [red.log](red.log)). La même fixture accepte 100, la borne de l’heuristique.

`ContextPacketV1::estimate_tokens_for_text` définit la formule générique `ceil(UTF‑8 bytes / 4)`. Le CLI `forgejo-mission-local` réutilise cette fonction au lieu de recopier la formule. `AgentExecutionRequestV1::validate` recalcule le plancher sur le texte reçu; toute valeur inférieure devient `context_token_estimate_underreported`, avec estimate, minimum, nombre d’octets et budget dans les détails. L’estimateur de texte du work item utilise lui aussi `usize::div_ceil(4)` pour le même arrondi, sans changement numérique.

La validation verte du test refusé et l’assertion que le producteur construit le manifeste avec la fonction partagée passent respectivement 1/1; les sorties sont dans [green-after-restore.log](green-after-restore.log) et [focused-tests.log](focused-tests.log). Les changements de code sont dans `src/agent_runtime.rs`, `src/bin/forgejo-mission-local.rs`, `src/lib.rs` et `tests/agent_runtime_contract.rs`.

## Vérifications

La suite `cargo test --offline --locked --all-targets -- --test-threads=1` termine avec code 0 : 728 réussis, 0 échec, 158 ignorés, 38 groupes de résultats. Le nouveau test figure dans le log intégral [all-targets.log](all-targets.log); les ignorés n’ont pas été exécutés par cette commande. `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` passent; le readback Cargo est [cargo-license-state.json](cargo-license-state.json).

Clippy strict all-targets (`-D warnings`) reste en échec, code 101, à cause de lints déjà présents dans le checkout modifié (argument counts, booleans, structures conditionnelles, etc.). Le log [clippy.log](clippy.log) ne signale ni `estimate_tokens_for_text`, ni `context_token_estimate_underreported`, ni les lignes modifiées du CLI; le résultat n’est pas revendiqué comme un Clippy vert. Le diff est space-clean.

## Portée et réserves

Cette garde lie l’estimate déclaré au texte selon une formule générique; elle n’est ni un compte exact du tokenizer d’un modèle, ni une borne universelle, ni une limite imposée par un fournisseur. Seul `context.text` est re-compté ici; un futur adaptateur devra compter tous les champs réellement envoyés et imposer le budget par le host avant l’appel. Aucun AgentOS, Forge Code, RAG, système de mémoire ou LLM n’a été utilisé dans ce lot. Aucun token fournisseur, coût ou gain n’est mesuré.

Le produit garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed` et `NOASSERTION`; aucun choix de licence n’a été fait. Le HEAD et le remote Forgejo loopback restent inchangés; aucun commit, publication, transfert GitLab, credential ou environnement de production n’a été utilisé. L’inventaire de fichiers source et sorties est dans [source-hashes.txt](source-hashes.txt); versions d’outils dans [tool-versions.txt](tool-versions.txt).
