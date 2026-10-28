# CONTINUATION-314 — PR source Forgejo réelle et admission requirements v3

Date : 27 octobre 2026, heure locale America/Montreal. HEAD du dépôt produit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree déjà fortement modifié. Aucun reset, commit, publication, changement de production ou appel modèle.

## Résultat observé

Le runner a lancé une instance jetable Forgejo v17.0.0, son receiver et le CLI d’admission. Il a créé le dépôt privé `forge-admin/mission-012-clamp`, l’issue réelle #1 et la branche `requirements/mission-012-v3`. La PR source d’exigences #2 a été approuvée par le compte local synthétique `verifier-bot` (ID 2, avis #1), puis fusionnée dans `main`. Le readback confirme la PR fusionnée, la tête source `feff85cc9cb42aa14d3416a4d1767d7e6bef7f93` et commit de fusion/tête courante `bf4c3951c732cf1c0dee8c7cb85dea76a2c5fafa`, ainsi que l’égalité octet par octet des exigences. SHA-256 des exigences : `a1b1280052c2c0d605afec66360b563f9817b41b63ec5d62572104da233379b6`; policy hôte révision 1 SHA-256 `748b8bd55fac7b7952f5a570fa46ae4572522139527343574b967f0d51a66363`, mode `0600`.

Forgejo a émis le webhook `issues/opened` réel. Le receiver l’a reçu (HTTP 202, un enregistrement); le CLI a réconcilié l’issue, puis réclamé l’admission schema 3, génération 1. La source d’exigences, le commit de base du workspace et le contexte sont liés au commit fusionné. Le contexte cite les exigences et `src/lib.rs` à ce commit, inclut le défaut réel du clamp et un budget estimé à 141 tokens. Le reçu indique `agent_runtime_invoked=false` : aucun agent n’a modifié le code.

Le reçu principal est `result.json`; les readbacks sont `requirements-source-pr.json`, `admission.json`, `branch-protection-readback.json`, `hook-readback.json`, `webhook-event.json` et `forgejo-task-binding.json`. Le reçu de nettoyage indique les processus arrêtés et les ports attendus absents.

## Échec et correction

Les essais précédents sont conservés. `...-001` a échoué sans code de décision. `...-002` a retourné `mission_requirement_arbitration_insufficient`; le readback DB indique un avis `APPROVED` mais `dismissed=1` avec `dismiss_stale_approvals=true`. `...-003` a passé l’admission mais n’a pas persisté le reçu source PR. `...-004` a retourné le même refus pour `stale=1,dismissed=0`, car le validateur rejetait alors tout avis stale.

Le run final a utilisé une branche protégée jetable avec une approbation requise, un merge whitelist owner-only et `dismiss_stale_approvals=false`. L’avis reste non dismissé mais devient stale. Le CLI accepte uniquement une revue `APPROVED` officielle, non dismissée, liée au commit exact de tête et à un ID autorisé; il compare aussi les exigences au commit de fusion et à la branche protégée courante. Un ancien head demeure refusé. Désactiver la révocation est une adaptation de test, pas une solution de production.

## Vérifications

- `cargo test --locked --offline --all-targets` : 720 réussis, 0 échec, 157 ignorés, en 38 blocs; les ignorés n’ont pas été exécutés.
- Tests Python du runner : 43/43; `py_compile` passe.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent.
- Garde Cargo : `license=null`, `no_license_selected`, `publish=[]`, droits `not_assessed`.

Commandes, versions, logs et limite de l’examen amont : `commands.txt`, `tool-versions.txt`, `cargo-test-all-targets.log`, `python-runner-tests.log` et `source-review.txt`.

Traçabilité des manifestes : la première exécution du helper a ajouté les empreintes du lot au manifeste local, puis s’est arrêtée avant les deux manifestes globaux parce qu’elle avait résolu le mauvais répertoire racine. Aucun manifeste global n’a alors été modifié. Le helper a été corrigé, le préfixe local reste vérifié contre `manifest-prefix-before.json`, et les changements successifs de helper sont gardés comme lignes append-only; le vérificateur contrôle le dernier digest par chemin.

## Limites et suite

Test d’admission des exigences `n=1`, pas le parcours complet : aucun PR de code, modification exécutable, vérificateur indépendant, décision de merge de code candidat ou test post-fusion du comportement. L’approbateur est synthétique et la policy vient du harnais. Forgejo, receiver et CLI partagent l’UID hôte; la liaison du hook lit une table SQLite interne non publique. Forgejo local est `17.0.0+gitea-1.22.0`, Go 1.26.9, commit de build inconnu. Aucun AgentOS, modèle, RAG ou mémoire n’a été appelé; aucune comparaison de coût, tokens, durée ou intervention humaine. GitSpace n’est pas utilisé et ses droits restent indéterminés; aucun code Forgejo/GitSpace n’est copié au produit.

Prochaine action : tester un reçu de revue source vérifiable après fusion sous la révocation normale, lié à la tête et aux octets protégés courants, puis traiter l’identité de l’approbateur et la course entre lectures. Le projet reste incomplet.
