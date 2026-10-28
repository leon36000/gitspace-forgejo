# MISSION-012 — rapprochement du prototype tool-operation et du produit

Date locale : 25 octobre 2026. Worktree produit : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Rust/Cargo 1.95.0. Aucun appel réseau, LLM, Forgejo, publication ou déploiement pendant ce contrôle.

## Résultat

La journalisation/reprise de `workspace.write_text` décrite précédemment est dans le snapshot ignoré `.runtime/mission-012-agent-pr-gate-004/`, pas dans les sources produit. Cinq fichiers clés ont des SHA-256 différents entre ce snapshot et le produit. Le snapshot contient les DTO et méthodes de journal d’intent/résultat, les hooks MCP et la précondition writer; le produit conserve aussi des évolutions qui n’existent pas dans le snapshot (notamment journal CPU cumulatif et autorité de merge). Aucun remplacement de fichier n’a été fait.

Une assertion TDD temporaire dans `tests/mission_012_mcp_lifecycle.rs` a demandé un intent puis un résultat durable pour le cycle MCP existant. Le run rouge a observé `write_intents.len() == 0`. L’assertion temporaire a été retirée; aucun test rouge ne reste dans le worktree. Après retrait, la suite MCP du produit passe `24 passed, 0 failed, 1 ignored`.

Le test de crash signalé par le snapshot n’est pas activable dans le produit : `cargo test --offline --locked --features test-crash-injection --test mission_012_mcp_lifecycle` retourne 101 avec `package 'agent-forge-core' does not contain this feature`. Le manifeste `Cargo.toml` confirme qu’aucune feature n’est déclarée. Ce résultat établit une absence de configuration, pas un défaut d’exécution de la feature.

## Sources comparées

| Fichier | SHA-256 produit | SHA-256 snapshot ignoré |
|---|---|---|
| `src/agent_runtime.rs` | `1948de1d635d9b7a436f2a3dfa470ffd2326d30fb92165c7d9d3a38baeb09030` | `d748e1be6c9e024693b8566938eb06fe6b13282ef0e944a439880bdcc540b64c` |
| `src/bin/mission-012-mcp-host.rs` | `979fd75c7a6c056a99b92d1ec57fb45bbf352bf43696559d1ee365b930e55527` | `e4795806625b7a64293df1d1b7cc66c84e315008d79b864b4cb43b4be18bb20f` |
| `src/mission_012_host.rs` | `312d9bb7eb135fc9b6bb59c616d47451fbd11f98c9668a192a09f3af9dc5f7c4` | `65e80a7c4b26edcdfcc25f41b7c004613034107ef3af5c54c88234976a68e5f4` |
| `src/lib.rs` | `369fb9f75bf40b6d4d8f6bb1a6519e6b174678a1786a553901910ca52b640c37` | `75d0c1f38194eea90ca20157e3e6e04c9d84ad679772f2917f161e5a1066fb02` |
| `tests/mission_012_mcp_lifecycle.rs` | `da39cf0b30e9a8bdaf95e81d39abeef48b56677b7951dc271de4acedf77c8f1e` | `f1e46ad2b822bf3033a928b9afc5f9e54947aae4a81c082be36338e1375135ae` |

The snapshot has no independent commit identity. These hashes identify the exact files compared; the parent HEAD is context only. Static review was limited to the files above and the test/report sections they reference. Historical test counts in the snapshot report were not rerun here and are not attributed to the product.

## Commandes et preuves reproductibles

```sh
cargo test --offline --locked --test mission_012_mcp_lifecycle
cargo test --offline --locked --features test-crash-injection --test mission_012_mcp_lifecycle
```

La première commande passe `24/0/1`; voir `product-mcp-lifecycle.log`. La seconde échoue au parsing des features, avant compilation; voir `crash-feature-unavailable.log`. Le test-first temporaire qui prouve l’absence d’intent est conservé dans `product-red-intent-contract.log`. `rustfmt --edition 2024 --check tests/mission_012_mcp_lifecycle.rs` et `git diff --check` ont aussi réussi. Le manifeste local valide ses 8 rapports, journaux, versions et empreintes; la sortie `8/8` est dans `manifest-check.log`. Le rapport vérifie une lacune d’intégration; il ne démontre pas la correction du protocole d’idempotence du snapshot.

Après mise à jour des empreintes des documents et du présent lot, la vérification du manifeste global a trouvé 2 846 lignes : 2 845 entrées valides, 0 fichier manquant, 0 divergence, et le seul placeholder historique malformé `None  tests/test_mission_012_live_composed_e2e.py`. Deux entrées antérieures portant sur `Cargo.toml` et `scripts/verify_cargo_license_state.py` ne correspondaient plus aux fichiers présents; leurs empreintes ont été recalculées sans modifier ces sources.

## Prochaine tranche

Porter sélectivement les transitions d’intent/résultat et leur validation dans `src/agent_runtime.rs`, en conservant les validateurs CPU et merge actuels; les réserver dans `src/lib.rs`; ensuite ajouter la précondition au writer et les hooks MCP avant d’activer/reproduire les cas SIGKILL. Revalider chaque étape sur ces sources exactes avant de déclarer la capacité intégrée. Ne pas remplacer les fichiers du produit par ceux du snapshot.
