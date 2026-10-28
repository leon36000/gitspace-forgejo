# FORGEJO-AGENT-STDIO-MCP-MERGE-001 — frontière de processus pendant une fusion protégée

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; aucun reset ni commit n’a été fait.

## Exécution et preuve

Commande locale, code de sortie 0 :

```text
python3 scripts/run_forgejo_live_pr_merge_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-agent-stdio-mcp-merge-002 --non-admin-merger --stdio-mcp-egress-during-merge
```

L’instance isolée utilisait Forgejo `17.0.0+gitea-1.22.0`; son binaire local a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. L’environnement vérifié est Python 3.12.3, Rust/Cargo 1.95.0 et Bubblewrap 0.9.0. Le listener 127.0.0.1:3130 était absent après l’arrêt normal du runner. La sortie complète, la base Forgejo et le `verifier.key` sont restés dans `/tmp`; seules les données dérivées sans secret sont conservées ici.

Le processus réel `--stdio-mcp`, lancé sous Bubblewrap avec namespaces réseau et PID distincts, a exécuté sa tâche Rust et a tenté d’atteindre les routes API, Web et `git-receive-pack` de l’instance hôte : les trois sondes ont été bloquées, `network_connected=false`, zéro connexion de route observée par le superviseur. Le child n’avait que les descripteurs 0, 1 et 2, aucun credential visible, et les clés d’environnement limitées à `HOME`, `LANG`, `LC_ALL`, `PATH`, `PWD` et `PYTHONDONTWRITEBYTECODE`. L’essai de lire le descripteur `/proc` du processus hôte a été refusé dans le namespace PID; le descripteur CLOEXEC du token de test n’était ni hérité, ni sérialisé dans la config/commande/environnement MCP, ni persisté dans un fichier. Le token local write-scope est resté dans la mémoire du parent et son memfd, tronqué puis fermé par le runner.

La tâche locale a d’abord produit un échec exploitable (`cargo test` exit 101 : valeur observée `-2`, valeur attendue `10`). Le processus a lu le contexte via MCP, a modifié `src/lib.rs` avec `workspace_write_text`, a repassé le test, puis a relu le contexte; le host refuse toute différence de contexte à ce point. Cette tâche est une fixture locale `local-test-adapter`, sans LLM ni AgentOS.

Pendant que ce processus restait vivant à la barrière MCP après acceptation, le gate hôte a relu la branche cible (HTTP 200, SHA stable avant/pendant la lecture), puis l’autorité Rust a soumis une fusion de la PR locale #2. L’autorité a lié la PR, le SHA de tête `11d2c0703ed4991bebd9fbb63f3723d1114923af` et la base validée `2e9b76760f03f1025cf8cf9fb9a37953abbf5a86`; l’API de fusion a retourné HTTP 200 et le verdict est `merged`. La tête `main`, le readback de fusion et le SHA de commit après fusion concordent (`45eaa653e311d69dce6238e99375442c33c343e0`); l’issue est close. Le vérificateur indépendant exécuté après fusion a testé le SHA source `d2d3ae071c330a25aa59fba851ef59d51c3bf6da63138953e87c96b2b6274c45`, identique à la source relue, avec 9 tests réussis et 0 échec. Le runner rapporte aussi le refus d’un faux succès publié par l’exécutant.

Les résultats assainis sont dans `composition-result.json`; hashes de sources, outils et artefacts de preuve sont dans `provenance.json`. Les deux sorties de la tâche locale, le vérificateur post-fusion et la suite Python sont archivés dans ce dossier. Les snapshots de code sont sous `source/`.

## Vérifications fraîches

- `python3 -m unittest discover -s tests -p 'test_*.py' -v` : 241 réussis, 0 échec, en 19,816 s.
- `python3 -m unittest -v scripts.test_cargo_license_state` : 9/9.
- `python3 -m py_compile` sur les cinq scripts et trois tests touchés : code 0.
- `git diff --check` : code 0.
- Le vérificateur de politique Cargo répond `status=verified`; `cargo metadata --no-deps` confirme `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed` et marqueur `NOASSERTION`. Aucune licence n’est choisie.
- Le manifeste local de ce lot et les entrées courantes des manifestes append-only produit/parent sont contrôlés dans `manifest-check.txt` et `global-manifest-check.txt`.

## Itération précédente

Le premier essai composé a échoué en mode fail-closed avant l’appel du gate : le helper utilisait encore le dépôt `mission-012-clamp`, absent du Forgejo de cette exécution (`GET /repos/forge-admin/mission-012-clamp/branches/main` → HTTP 404). Le runner a été paramétré avec la cible owner/repository/PR et la lecture de branche courante, puis l’exécution complète a réussi. Détail et hash d’erreur dans `iteration-history.md`.

## Portée et limites

Cette preuve établit la séparation du processus MCP réel et du credential du gate pour ce parcours local, pendant qu’une lecture et une fusion autorisées fonctionnent. Le travail que le processus MCP a écrit est une fixture Rust distincte du candidat contenu dans la PR #2 : ce n’est donc pas la démonstration que l’agent de cette tâche a conçu le changement fusionné. Aucun webhook n’est livré/réconcilié dans cette même exécution; le parcours séparé [FORGEJO-PR-MERGE-WEBHOOK-002](../forgejo-pr-merge-webhook-002/local-run.md) reste une preuve complémentaire, sans le rejouer.

La frontière dépend ici de Bubblewrap, des namespaces disponibles et de cette configuration. Forgejo et le runner partagent le même UID; aucune garantie contre évasion noyau, acteur privilégié ou autre configuration d’hôte n’est démontrée. Le serveur n’exclut pas les autres writers (`external_writers_excluded=false`); cette réussite ne ferme pas la course de base contre des écritures concurrentes directes. Les sondes ne valent pas audit exhaustif des routes Forgejo. Ce parcours n’utilise aucun AgentOS, aucun LLM, aucune CI conventionnelle comparée et ne mesure aucun gain; l’effectif de ce nouveau run est n=1. Aucun secret n’a été envoyé à un service externe, aucune dépense nouvelle, production, publication ou modification serveur Forgejo n’a été faite. Le projet reste incomplet.
