# ADMISSION-HOST-HANDOFF-002 — parcours frais CLI → MCP → PR

Date : 24 octobre 2026. Worktree local `.runtime/webhook-005-workspace`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les sources applicatives sont non commitées dans ce worktree; le snapshot `provenance.json` lie chaque fichier source pertinent à son SHA-256. Toolchain Cargo/Rust 1.95.0. Aucun fichier de production, dépôt distant ou service externe n’a été modifié.

## Parcours exécuté

Commande reproductible depuis la racine du worktree :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-041 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-041" \
  --execution-id live-mcp-041 \
  --allocation-id mission012-local-adapter-041 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

Le runner est sorti avec le code 0. Il a démarré un Forgejo local v17.0.0 (`gitea-1.22.0`), binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, dans un répertoire d’état privé temporaire. La provenance du build de ce binaire depuis le clone officiel n’est pas établie.

Le parcours observé est : livraison réelle de webhook Forgejo `issues/opened` → admission et claim idempotents du CLI → écriture du DTO privé → hôte MCP versionné → contexte et appels d’outils → échec du test d’acceptance sur la base → correction → vérification indépendante → refus de fusion avant revue → PR relue → fusion par l’autorité Rust → readback Forgejo. Le même SHA-256 du fichier d’admission apparaît dans les étapes admission et agent : `aab1d871538d74866123f96e99ffefe8a1a069a7947dadad4294ab196ee286e9`. Le binaire hôte MCP construit pour ce run a le SHA-256 `e96e62f5b0330b341b390258ec26ff8ad37ac46c3d83e9f87820e2cc1160a745`.

L’acceptance échoue d’abord sur `clamp_to_range(15, -2, 10)` (`-2` au lieu de `10`), puis les deux tests passent après la correction. Le candidat change uniquement `src/lib.rs`; son blob SHA-256 est `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`. Le vérificateur indépendant retourne `pass` sur le candidat. Forgejo refuse la fusion avant revue et la tentative du reviewer de test, toutes deux HTTP 405. L’autorité `forgejo-reconcile-local-rust` fusionne ensuite le candidat `a83a90eb405b364861c6ce0441ae23929812beee` au commit `c1f984f4411f6c289f88472407fe4b60a2f77c15`; l’issue est relue fermée. Les trois notifications PR ont chacune une tâche HTTP 202 et un reçu lié à l’état courant fusionné. Le hook est désactivé avec readback et le receiver est arrêté.

## Sources et preuves

- Le CLI construit `MissionHostAdmissionV1` depuis le reçu d’admission dans `src/bin/forgejo-mission-local.rs::admission_output`; l’option `--host-admission-file` écrit les octets canoniques dans un fichier privé, avec contrôles de type, propriétaire, permissions, liens, rejeu, conflit et synchronisation disque.
- `src/mission_admission.rs::MissionHostAdmissionV1::validate` vérifie les égalités commit, issue, chemin, hash d’exigence, requête et allocation. `src/bin/mission-012-mcp-host.rs` consomme ce DTO et relie la requête au profil MISSION-012 et au journal durable avant d’ouvrir une lease MCP.
- `evidence/mission-012-live-agent-webhook-local-adapter-041/composed-stage-summaries.json` lie les hashes d’admission, d’agent et de fusion. `base-red-cargo-test.log`, `candidate-green-cargo-test.log`, `independent-verifier.json`, `pre-review-merge-refusal.json`, `premerge-reverification.json`, `rust-merge-authority-result.json`, `forgejo-result.json` et les reçus PR conservent les observations détaillées.
- `evidence/mission-012-live-agent-webhook-local-adapter-041-admission/provenance.json` lie le HEAD, les hashes SHA-256 de tous les fichiers Rust/Python consommés, les binaires construits et le binaire Forgejo. Le manifeste des preuves d’admission a le SHA-256 `d2cb112fc51dd88f48a2e28d8bdfe7de19f94c7785d22ef7ca04d18cb311e21e`.
- Les commandes `sha256sum -c SHA256SUMS` dans les deux répertoires du run et de l’admission passent.

## Portée et limites

Ce run emploie explicitement `local-test-adapter`, sans appel LLM et avec coût modèle rapporté de 0 USD. Il démontre le raccordement processus Forgejo → CLI → fichier DTO → processus MCP → PR et autorité de fusion pour une seule tâche fixture; il ne mesure aucun gain général. La CLI Claude 2.1.310 est disponible localement et un run Opus 5.5 distinct est documenté dans le checkpoint, mais l’identité du modèle effectivement servi n’est pas attestée par Anthropic.

Le DTO n’est pas signé et n’atteste pas à lui seul que son origine était un webhook plutôt que le mode d’admission local direct. Le journal et les fichiers partagent le même UID; un processus hostile du même UID n’est donc pas exclu. Le readback empêche de confondre les livraisons de revue avec l’approbation, mais l’API Forgejo ne fournit pas ici de CAS contre un autre writer concurrent de la branche. Le binaire Forgejo est identifié par hash, pas par une provenance de construction. AgentOS, Forge Code, RAG et mémoire ne sont pas intégrés dans ce parcours. Les droits GitSpace restent indéterminés et `license=null` demeure non choisi.

Cette vérification fraîche complète le rapport préliminaire ADMISSION-HOST-HANDOFF-001, qui est obsolète sur l’absence d’émission du DTO par le CLI. Le worktree reste non commité; ne pas attribuer au commit HEAD les sources non suivies.
