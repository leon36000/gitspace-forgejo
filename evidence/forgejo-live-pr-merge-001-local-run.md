# FORGEJO-LIVE-PR-MERGE-001 — issue, PR, vérification et fusion locale

## Résultat observé — run 07, 23 octobre 2026

Le run 07 a utilisé une instance neuve Forgejo `17.0.0+gitea-1.22.0` sur `127.0.0.1:3130`, dépôt SQLite et comptes temporaires sous `/tmp/forgejo-live-pr-hardened-20261023-07`. L’instance est arrêtée. Après le health check, `ss` a confirmé que le seul listener sur le port appartenait au processus Forgejo démarré par le harnais. Aucun service de production ni dépôt antérieur n’a été utilisé.

L’issue #1 a conduit à la PR #2 dans `forge-admin/agent-forge-core`. Le premier candidat renvoyait sa valeur sans appliquer les bornes : 4 cas passaient et 5 échouaient. L’adaptateur déterministe a appliqué `value.clamp(min, max)` après ce retour exploitable. Le worker final a ensuite réussi les neuf cas host-owned : bornes inférieure et supérieure, valeur intérieure, égalité aux bornes, intervalle singleton, valeurs négatives et extrêmes `i32`. Le candidat synthétique qui appelle `process::exit(0)` sans sortie a été refusé sur 9/9 cas. Aucune exécution LLM n’a participé à ce parcours.

Forgejo a rapporté une seule modification (`src/lib.rs`) lors de deux lectures paginées complètes des fichiers de la PR : à sa création et avant la fusion. Les octets lus au SHA tête `2fc1d800473ed828ad152f04cfd0cb250faf7807` ont la somme SHA-256 `d2d3ae071c330a25aa59fba851ef59d51c3bf6da63138953e87c96b2b6274c45`. Le worker binaire exécuté a la somme `94347097f1c48eb26b881b494475284b2c9b924281f1b50571641a0ddaff4dd6`.

La preuve de la version précédente a été rejetée `stale_candidate`; le préflight de la version vérifiée a renvoyé `proof_current`, sans autoriser ni tenter de merger. La règle relue cible `main`, exige une revue, limite l’approbateur à `forge-reviewer` et le mergeur normal à `forge-admin`. Sans revue, Forgejo a refusé le merge HTTP 405 avec `Does not have enough approvals`; après approbation, le jeton du reviewer hors liste a reçu HTTP 405 `User not allowed to merge PR`. Les deux tentatives ont laissé la PR ouverte. Le merge normal a ensuite renvoyé HTTP 200. La PR et l’issue sont fermées, le commit de fusion et la tête de `main` sont `920b269411db19fbc4369abae1fdc9c678cf0738`, et la source après fusion égale la source testée.

Le [manifeste SHA-256](forgejo-live-pr-merge-001-sha256.txt) couvre le runner, les entrées de build, les pièces run 07, le rapport et le binaire Forgejo identifié.

Une revue indépendante en lecture seule du hash exact du runner n’a trouvé aucun défaut matériel dans les catégories proxy, isolation du build/candidat, liaison entre head testé et preuve, assertions de protection, refus du faux succès et validation du résultat rouge. Son périmètre et ses limites sont dans [la revue run 07](forgejo-live-pr-merge-001-run07-review.md); cette revue n’a pas relancé l’exécution. Un audit séparé Claude Opus/Sonnet du même lot est en cours et sera ajouté après réception.

Les read-backs expurgés et les sorties du run sont conservés dans [les résultats run 07](forgejo-live-pr-merge-001-run07-result.json), [le préflight périmé](forgejo-live-pr-merge-001-run07-stale-preflight.json), [les read-backs avant et après merge](forgejo-live-pr-merge-001-run07-premerge-readback.json) et [le read-back final](forgejo-live-pr-merge-001-run07-postmerge-readback.json). Les tests rouges, verts et le faux succès sont dans [les sorties du vérificateur](forgejo-live-pr-merge-001-run07-red-verifier.log), [succès](forgejo-live-pr-merge-001-run07-green-verifier.log), [vérification finale](forgejo-live-pr-merge-001-run07-final-verifier.log) et [faux succès refusé](forgejo-live-pr-merge-001-run07-fake-success-verifier.log).

## Provenance et frontières réellement testées

Le binaire Forgejo local est identifié comme `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Ce hash identifie le fichier, pas sa provenance de téléchargement. La source Forgejo examinée est le tag officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. L’examen ciblé a couvert les handlers REST de protection de branche et de merge, le modèle de protection, le contrôle de merge, les files de PR et les tests de scopes d’API. Dans `tests/integration/api_repo_test.go:835`, le test Forgejo v17 vérifie qu’un jeton `write:repository` seul ne peut pas créer `/user/repos`; les essais locaux ont aussi confirmé que `write:user` seul est refusé. Le jeton temporaire de provisioning porte donc `write:user,write:repository` pour ce seul POST. Le runner abandonne sa référence et remplace l’opener par un jeton `write:repository,write:issue` pour le reste du parcours. Le handler de refus de merge et les contrôles d’approbation se trouvent dans `routers/api/v1/repo/pull.go`, `services/pull/check.go` et `models/issues/pull.go` du même tag.

Le build des exécutables Rust `gate` et `forgejo-reconcile-local` est `--offline --locked`, dans Bubblewrap sans réseau et sans home hôte. Le harnais ne stage que les sources Cargo choisies et le registre verrouillé; les hashes des arbres source/registre, des deux exécutables, du harnais, du binaire Forgejo, des versions Rust/Cargo et de Bubblewrap sont dans [la provenance du build](forgejo-live-pr-merge-001-run07-trusted-build-provenance.json). L’appel local désactive les proxies d’environnement. La preuve signée lie les exigences et la politique, base et tête, hash des sources exécutées, hash du worker, transcript du vérificateur et hash des chemins modifiés.

Le source candidat est compilé dans un worker distinct. Chaque cas lance ce worker dans un nouveau namespace réseau/PID, avec seulement son binaire monté en lecture seule; le code de test et les attentes restent dans le processus hôte du vérificateur. Les preuves sont signées et vérifiées sous Bubblewrap sans réseau. La clé HMAC, la base, les journaux bruts du serveur et les tokens ne sont pas copiés au dépôt; seuls des read-backs expurgés et des transcriptions de test y sont conservés.

## Reproduction

Avec un nouvel emplacement direct sous `/tmp` et le port 3130 libre :

~~~sh
python3 -m py_compile scripts/run_forgejo_live_pr_merge_e2e.py
python3 scripts/run_forgejo_live_pr_merge_e2e.py \
  --forgejo-binary /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.tools/forgejo-17.0.0-linux-arm64 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-live-pr-hardened-<nouveau-nom>
~~~

Commande réellement exécutée : `python3 scripts/run_forgejo_live_pr_merge_e2e.py` avec le binaire identifié ci-dessus et `--state-dir /tmp/forgejo-live-pr-hardened-20261023-07`; elle a terminé avec code 0. Le script a aussi passé `python3 -m py_compile`. Les versions relevées sont Python 3.12.3, Rust/Cargo 1.95.0 et Bubblewrap 0.9.0.

## Couverture et limites

Le harnais Python détenait les identifiants temporaires et a exécuté lui-même les appels de provisioning, review et merge; ce n’est pas une autorité de fusion de production. Le reviewer et le faux candidat sont synthétiques. La tâche a été fournie par l’adaptateur déterministe; aucun agent LLM, AgentOS, RAG, mémoire, CI conventionnelle ou réviseur humain n’a participé au parcours. Un seul changement logiciel a été exécuté; ce run ne mesure pas d’amélioration sur la qualité générale, le temps, les interventions, les tokens ou le coût.

### Relation avec le parcours LLM MISSION-012

Un essai distinct du 22 octobre, documenté dans [MISSION-012](mission-012-local-run.md), a utilisé Claude Code `claude-opus-5-5` en effort `high` via le host MCP local pour corriger un autre petit fixture Rust, puis l’a fait vérifier et fusionner dans une autre PR Forgejo locale (#4). Son modèle, commits, issue et instance diffèrent de run 07. MISSION-012 lie ses preuves de test aux commits, mais l’association portable du rapport de vérification à l’identité Forgejo reposait en partie sur le read-back manuel de l’opérateur. Le run 07 durcit et observe les règles Forgejo, les refus API, la fraîcheur et la fusion contre une instance neuve, mais emploie un adaptateur déterministe. Ces deux résultats ne constituent donc pas une exécution composée unique ni une intégration de production d’AgentOS/RAG/mémoire. Le prochain parcours utile doit joindre l’admission/claim, le contexte autorisé, l’agent LLM et l’adaptateur Forgejo derrière une seule autorité hôte, puis refaire le preflight au moment de fusion.

La vérification `head/base` est suivie d’un POST normal avec comparaison de la tête candidate, mais Forgejo v17 ne donne pas ici de compare-and-swap atomique sur la base. Une course sur la base après le dernier read-back n’a pas été injectée. Les routes admin `force_merge`/`manually-merged`, la reprise du processus pendant le merge, les webhooks dupliqués et les changements concurrents sémantiques ne sont pas exercés par ce scénario. Il n’y a pas de limite mémoire agrégée cgroup ni de test contre un acteur hostile du même UID; Bubblewrap ne constitue pas une frontière de sécurité noyau. Le staging local garde une fenêtre TOCTOU entre certaines validations et copies.

La tête du worktree d’intégration est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le runner est un fichier non suivi et le worktree contient d’autres changements antérieurs. Les hashes de build enregistrent les entrées effectives du run, qui ne correspondent donc pas à un commit unique du dépôt. Aucun commit, push, publication ou changement de production n’a été réalisé.
