# MERGE-RECOVERY-001 — reprise du merge et parcours Forgejo local

Date : 24 octobre 2026. Dépôt de travail au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree contenait déjà de nombreuses modifications locales. Aucun commit, push ou déploiement n’a été effectué.

## Résultat

Le journal de mission persiste l’intent avant le POST de merge et rend les reprises readback-only. Le run composé **039** confirme, sur le code actuel, le parcours d’une petite correction Rust dans une instance Forgejo privée jetable : événement webhook → admission versionnée → contexte ciblé → test rouge → modification de `src/lib.rs` → PR → vérification indépendante → revue → autorité Rust → fusion et readback. La branche candidate `4522b30ae8a42ce36d09ead32bef7caf200a5d61` corrige le clamp : sur la base `440088fd055e0bfcd50177a32f2e3c91cf60bbca`, un test d’acceptation attendait 10 et obtenait -2; le même test passe sur le candidat. Le commit de fusion lu sur `main` est `7d6d72a5e567e837e5a2f82fa626a536c404b431`, PR #2, issue fermée, seul chemin modifié `src/lib.rs`.

Commande reproductible, sans secret en argument :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-039 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-039" \
  --execution-id live-mcp-039 \
  --allocation-id mission012-local-adapter-039 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le résultat du composé lie les hashes d’exigence, admission, candidat, policy, revue, receipts et readback; le vérificateur relance l’acceptance du commit exact dans Bubblewrap, offline/locked, avec snapshot read-only et credentials absents. Les tentatives de merge avant revue et par le reviewer sont refusées HTTP 405. L’autorité de fusion invoquée est `forgejo-reconcile-local-rust`.

`local-test-adapter` est un adaptateur déterministe de test; aucun modèle n’a été appelé (`model_invoked_during_run=false`, coût modèle déclaré 0). Ce run valide les interfaces et le comportement de la fixture, pas un agent LLM. Le détail signé par hashes et les pièces sont dans [le dossier d’évidence 039](../mission-012-live-agent-webhook-local-adapter-039/composed-run.json); les manifestes du run et de son admission passent `sha256sum -c`.

## Reprise après interruption

`cargo test --offline --locked --test mission_execution_journal_contract` : 53 réussis, 0 échec, 2 ignorés. `cargo test --offline --locked --all-targets -- --test-threads=4` : 404 réussis, 0 échec, 17 ignorés, 33 groupes (`all-targets-final.log`). Le runner `scripts/run_forgejo_mission_delivery_process_e2e.py` exécute sept cas sous Bubblewrap avec namespaces réseau et PID; les sept passent. Trois cas lancent le vrai binaire `forgejo-reconcile-local` sous une fixture HTTP stateful qui implémente un sous-ensemble du protocole Forgejo (ce n’est pas Forgejo) : après commit du POST et perte de réponse, deux reprises concurrentes restent en GET et il n’y a qu’un POST; après intent durable mais avant POST, l’état ouvert reste inconclusif et aucune reprise ne poste; une mutation de la policy pendant la dernière lecture Forgejo refuse le merge sans POST. Le doublon d’une résolution terminale ne fait pas de réseau. Une résolution legacy `inconclusive`, même si elle porte l’ancien `terminal=true`, bloque la nouvelle génération jusqu’au readback; sa résolution terminale de reprise utilise une nouvelle clé d’idempotence. Les logs isolés contiennent HEAD, hashes des snapshots source, du runner et du lockfile, ainsi que les versions Rust et Bubblewrap : [runner isolé final](isolated-process-e2e-final.log), [test de mutation des entrées](protected-input-process-e2e.log).

`cargo clippy --offline --locked --all-targets` et le build release passent sans erreur; Clippy émet encore des warnings de style/nombre d’arguments dans d’autres fonctions. `rustfmt --check` ciblé et `git diff --check` passent; les commandes et sorties sont enregistrées sous ce dossier.

Empreintes de l’essai isolé final : snapshot source `f039e71c601741139489a9e660f0ceca845dd955da9f5530f73e33c9710861c2`, runner `c29eb6016332c54dc4b284870317cb14b3c70252b6cccfb86cae5d08802e1bbf`, `Cargo.lock` `bf599559dd11ff21ecc4754aa8a693966b4a9ca401153d7d7caa1f18d8995fcc`. Outils : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`. Les sorties, le présent rapport et leurs empreintes sont recensés dans [SHA256SUMS](SHA256SUMS).

## Limites et prochaine action

La fixture de crash n’est pas le serveur Forgejo; le run 039 ne simule pas de crash sur Forgejo. Le verrou de journal sérialise les writers coopératifs, mais l’API merge ne compare pas atomiquement la base attendue contre un writer Forgejo externe; un processus local hostile du même UID peut aussi contourner les fichiers et verrous. L’empreinte prouve une liaison/intégrité des entrées, pas la correction du logiciel. Le run est `n=1`, sans LLM ni baseline appariée : il ne mesure aucun gain de réussite, interventions, durée ou coût. AgentOS, Forge Code, RAG et mémoire ne sont pas raccordés à cette tranche; les droits de réutilisation GitSpace restent à clarifier.

Suite utile : comparer sur des tâches appariées la CI conventionnelle et la couche agentique, avec modèle et budget explicitement contrôlés; auparavant, traiter la course de base avec un writer Forgejo non coopératif et borner le coût avant toute nouvelle invocation LLM. La session Claude/Ralph PID 1947069 a été surveillée deux fois en lecture seule, sans preuve de progression ni de blocage; elle n’a pas été arrêtée ou relancée.
