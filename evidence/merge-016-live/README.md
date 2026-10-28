# MERGE-016 — parcours Forgejo réel dans le bac local

Date : 23 octobre 2026. Deux runs ont relié la preuve V2 MERGE-016 à des instances Forgejo locales réellement démarrées, sur loopback et dans des répertoires temporaires privés. Le premier a exécuté le candidat isolé; après report du code et du runner dans W, le second a exécuté ce worktree. Ils complètent les tests Rust/HTTP isolés; ce ne sont ni un déploiement ni une démonstration LLM.

## Code et outils exécutés

- Le code candidat a été développé dans `.runtime/merge-016-target-bound-proof-sandbox`, qui n’a pas de commit Git propre. Après comparaison des empreintes, les quatre sources Rust et le runner ont été reportés dans le worktree W `.runtime/webhook-005-workspace`. W est toujours sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; ces changements sont locaux, non commités et non publiés.
- `src/lib.rs`: `8b96e80b165651f996946d93b691b5804735412288cf8dece224a815a9840974`.
- `src/bin/forgejo-reconcile-local.rs`: `c9cac988a401943e03ec940e59a46bbbdefa24250e88d55b9a201bbb81625f1d`.
- `src/bin/gate.rs`: `4366abaa55d4d5d539b10d48a3706afbf37702c9dd199466545963dbdc9d116d`.
- Runner E2E présent maintenant dans W: `e11b9d925a0b7bffa5cfb749788d088dbb13225472b0317690a5246474ffdee1`; son helper de staging/processus est inchangé, SHA-256 `67137cff9664745bbe8ec794b2d24d0c66debb8802409edda7178f007511df2b`.
- Forgejo `17.0.0+gitea-1.22.0`, binaire local ARM64, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- Rust/Cargo `1.95.0`, Bubblewrap `0.9.0`. Le client et le gate ont été compilés sous Bubblewrap sans réseau, avec Cargo `--offline --locked` et un arbre de sources/registre mis en staging et haché. Voir [`trusted-build-provenance.json`](trusted-build-provenance.json) et [`cargo-build.log`](cargo-build.log).

## Parcours et résultat observé

1. Le driver a créé l’issue Forgejo #1 et la branche `agent/clamp` avec une première implémentation réelle de `clamp_inclusive` qui renvoyait directement la valeur.
2. Le vérificateur d’acceptation indépendant a échoué sur les cinq bornes, tout en gardant quatre cas verts : [`red-verifier.log`](red-verifier.log). Un exécuteur qui retourne seulement un code de sortie zéro a aussi été rejeté : [`fake-success-verifier.log`](fake-success-verifier.log).
3. L’adaptateur déterministe de test a appliqué le correctif suggéré par l’échec (`value.clamp(min, max)`). Ce n’est pas une génération par LLM. La correction a passé 9/9 cas sur l’octet de source relu de Forgejo : [`green-verifier.log`](green-verifier.log), [`final-verifier.log`](final-verifier.log).
4. Le driver a ouvert la PR #2, créé une preuve V2 signée dans une sandbox hors réseau, et lié son MAC au profil `merge-014-agent-forge-core`, à `http://127.0.0.1:3130`, au dépôt `forge-admin/agent-forge-core` et à la base `main`. Le préflight V1 est resté consultatif et en lecture seule. Les fichiers [cible](final-proof-target.json), [preuve V2](final-proof-proof-v2.json), [évidence](final-proof-evidence.json) et [binding attendu](final-proof-expected.json) sont conservés; la clé de vérification n’est pas copiée.
5. Le gate Rust a refusé la fusion sans revue (`review_approval_missing`, aucune tentative de merge). Forgejo a aussi refusé le merge API sans revue (405). Une revue officielle du compte synthétique `forge-reviewer` a été liée au SHA candidat. Le gate exécuté sous le compte reviewer non autorisé a échoué fermé en `inconclusive/upstream_forbidden`, sans tentative de merge; Forgejo a refusé son POST (405).
6. Après une nouvelle lecture du candidat, de la base, des protections et de la revue, le gate a fusionné la PR autorisée (HTTP 200). Le résultat Forgejo et le SHA de `main` après fusion sont `f809d1484c65351daf4a16ef34f683e3c013b44c`; l’issue #1 est fermée. Le read-back confirme que les octets de `src/lib.rs` fusionnés sont ceux vérifiés.

Le résumé complet avec les SHA des commits du dépôt fixture est [`result.json`](result.json). La preuve signée elle-même est une attestation d’intégrité/provenance par possession de clé, pas une preuve de correction du logiciel.

## Vérification après report dans W

Les sources ont été reportées uniquement après comparaison des empreintes initiales de W avec la base MERGE-014 connue; chaque fichier destination a été haché après remplacement. Les quatre SHA listés plus haut sont ceux du candidat et de W. Les fixtures `policy/` et `requirements/` n’ont pas été remplacées. Le runner V2 a également été comparé à la version W et le helper a été confirmé identique.

Dans W, `cargo test --offline --locked --all-targets -- --test-threads=4` passe : 32 suites, 350 réussis, 0 échec et 12 ignorés. Clippy all-targets avec les trois allowances historiques, `rustfmt --edition 2024 --check` sur les quatre fichiers Rust et `git diff --check` sur les fichiers modifiés passent. Journaux : [`w-integrated-all-targets.log`](w-integrated-all-targets.log), [`w-integrated-clippy.log`](w-integrated-clippy.log). Le build release all-targets a passé dans la copie de validation avec les mêmes quatre sources et les mêmes fixtures hachées; voir `../merge-016/README.md`.

Le second run a exécuté le runner depuis W dans l’instance `/tmp/forgejo-merge016-v2-20261023-b`. Il retrouve le même comportement : rouge 5/9, vert 9/9, refus d’une preuve périmée, d’un faux succès d’exécuteur, d’une absence de revue et d’un reviewer non autorisé; fusion V2 avec HTTP 200 et read-back concordant. Pour ce run, le commit de base fixture est `b8cd6972aacd3c0137a260287922fae64239b661`, la tête candidate `a939b5e3a0cf6f5f474acce8c28dff6153610857` et le commit de fusion relu sur `main` `43353e52a8fe4a51f46fb60e2db104a9e2ca0f25`. Les preuves filtrées du second run et son manifeste sont dans [`w-run/`](w-run/), en particulier [`w-run/result.json`](w-run/result.json) et [`w-run/SHA256SUMS`](w-run/SHA256SUMS). Sa base et sa clé V2 restent dans le répertoire `/tmp` privé, jamais dans l’archive.

Commande du run W :

```sh
python3 scripts/run_forgejo_live_pr_merge_e2e.py \
  --forgejo-binary /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.tools/forgejo-17.0.0-linux-arm64 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-merge016-v2-20261023-b
```

## Reproduction et conservation

Commande exécutée depuis le dépôt candidat :

```sh
python3 scripts/run_forgejo_live_pr_merge_e2e.py \
  --forgejo-binary /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.tools/forgejo-17.0.0-linux-arm64 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/.runtime/merge-016-target-bound-proof-sandbox \
  --state-dir /tmp/forgejo-merge016-v2-20261023-a
```

Le runner n’autorise qu’un nouvel enfant direct de `/tmp` et refuse un chemin déjà utilisé. Les deux instances ont été arrêtées par le runner; les ports 3130/3132 étaient libres après chaque run. Les états sont conservés sous `/tmp/forgejo-merge016-v2-20261023-a` et `…-b`, chacun en mode `0700`; leurs sous-répertoires d’évidence sont `0700` et les clés V2 `0600`. Ces répertoires incluent aussi les bases et logs Forgejo. Aucun fichier n’a été supprimé. Les 26 artefacts copiés de chaque run ont été filtrés par liste explicite; clés, bases Forgejo et logs serveur sont exclus. [`SHA256SUMS`](SHA256SUMS) couvre le premier run, [`w-run/SHA256SUMS`](w-run/SHA256SUMS) le second.

## Limites

- Ce run est `deterministic local test adapter; no LLM configured or invoked`. Le parcours Claude Code documenté ailleurs utilise un autre driver; leur intégration de bout en bout n’est pas démontrée.
- Une seule tâche distincte a été rejouée deux fois; aucun bras Forgejo + agent + CI conventionnel n’a été exécuté et aucun gain en qualité, interventions, temps, tokens ou coût n’est établi.
- La fusion est un profil CLI local de test et l’instance est mono-utilisateur. Le verrou coordonne les appelants coopérants du même UID; Forgejo v17 n’offre pas de CAS de base dans cette requête de merge. Un écrivain externe peut donc faire avancer `main` après la dernière lecture. Aucun runner de production ou frontière de privilèges multi-tenant n’est revendiqué.
- Ce parcours ne teste pas l’interruption/reprise du pipeline complet, les doublons/désordres de webhooks ni les conflits sémantiques multi-agent. Les tests unitaires et d’intégration existants couvrent certains refus et reprises au niveau de leurs composants, pas ces scénarios dans cette instance live.
