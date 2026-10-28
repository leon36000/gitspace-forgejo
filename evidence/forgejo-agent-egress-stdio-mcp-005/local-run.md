# CONTINUATION-053 — processus d’édition isolé pendant une requête de gate

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le dépôt reste fortement modifié; aucune modification existante n’a été remise à zéro ou commitée.

## Question vérifiée

Les preuves précédentes vérifiaient soit le processus `--stdio-mcp` contre un listener synthétique, soit les routes d’une instance Forgejo réelle depuis un mode probe-only. Ce run compose le vrai processus de tâche `mission_012_test_agent_adapter.py --stdio-mcp` avec une instance Forgejo locale et une lecture de gate exécutée pendant que le child attend sa réponse MCP. Il ne rejoue pas de fusion Forgejo.

## Commande et environnement

```text
python3 scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-agent-egress-stdio-mcp-005 --evidence-dir "$PWD/evidence/forgejo-agent-egress-stdio-mcp-005"
```

Forgejo annonce `17.0.0+gitea-1.22.0`, construit avec Go 1.26.9; SHA-256 du binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Environnement : Linux 7.0.0-1019-nvidia aarch64, Bubblewrap 0.9.0, Python 3.12.3, Cargo/rustc 1.95.0. Les sources réellement lancées et les sources fixture sont recopiées sous `source/`; leurs empreintes figurent dans `provenance.json` et le manifeste local.

## Résultats

- Depuis l’hôte, Forgejo répond à `GET /api/healthz` (200), au dépôt API (200) et à sa page Web (200). Les trois routes ciblées répondent depuis l’hôte : PR API (404, car aucune PR #1 n’existe), PR Web (404) et `git-receive-pack` non authentifié (401).
- Le processus `--stdio-mcp` utilise Bubblewrap `--unshare-net --unshare-pid`, vise le vrai port Forgejo `127.0.0.1:3130` et rapporte les trois routes `api`, `web` et `git_receive_pack` bloquées, `network_connected=false`, un namespace réseau différent de l’hôte, les seuls descripteurs `0,1,2` et l’absence de lecture du fichier canari. Le superviseur n’observe aucune connexion au listener synthétique. Les contrôles hôte positifs distinguent un refus d’accès d’un serveur simplement arrêté.
- Une barrière contrôlée bloque l’échange sur `tools/call:mission_context`. Le processus demeure vivant pendant que le parent de confiance fait `GET /api/v1/repos/forge-admin/mission-012-clamp/branches/main`; l’API répond 200, le SHA de `main` est identique avant, pendant et après cette lecture, puis le MCP host libère le child. Le credential Forgejo limité à `read:user,read:repository` reste dans le processus parent et n’est transmis ni au child ni au MCP fixture.
- Le processus consomme le contexte par MCP, observe un vrai échec de `cargo test --offline --locked` (`left: -2`, `right: 10`), remplace `value.minimum` par `value.maximum` via `workspace_write_text`, puis le même test d’acceptation passe. Les sorties Cargo complètes, la source initiale et le candidat final sont conservés dans ce dossier.
- La suite `tests.test_mission_012_sandboxed_test_adapter` passe 7/7; `python3 -m unittest discover -s tests -p 'test_*.py' -v` passe 239/239 en 19,869 s. `py_compile` des quatre scripts concernés et `git diff --check` passent. Le test ciblé qui a précédé la modification a échoué comme prévu avec `TypeError: bubblewrap_command() got an unexpected keyword argument 'forgejo_probe_port'`; le test vert vérifie ensuite le port transmis à la commande réelle.
- Le runner arrête l’instance qu’il a lui-même lancée : code 0 et absence du listener 3130. Le manifeste local et les deux manifestes globaux sont vérifiés par le script joint.

## Frontière de credential et limites

Le host contient seulement un canari synthétique pour la sonde; aucun vrai credential de fusion n’est créé. La lecture de gate utilise un token local en lecture seule qui n’est remis ni au child ni au MCP host. Le résultat montre donc une séparation de namespace, de fichiers montés, de descripteurs et de credential lecture seule; il ne prouve pas la non-divulgation d’un vrai credential de fusion qui n’existe pas dans ce test.

Le processus agent est `local-test-adapter`, pas un LLM/AgentOS; le MCP host est une fixture locale allowlistée et n’appelle pas de service externe. Le gate effectue une lecture seulement : pas de PR ni de merge. Les cas API/Web/Git testés visent cette instance et cet hôte; ce n’est pas une preuve universelle d’absence d’egress, d’isolation multi-tenant ou de résistance à une évasion noyau. Forgejo, le superviseur et l’instance de test partagent le même UID local.

Une première invocation préliminaire a été rejetée avant le démarrage de Forgejo parce que la redirection de sortie avait prérempli son répertoire d’evidence; elle est conservée et expliquée sous `evidence/forgejo-agent-egress-stdio-mcp-001/attempt-note.md`. Elle n’a créé aucune instance et ne change pas le verdict du run 005.

Ce lot ferme la composition locale « processus réel d’édition + routes Forgejo réelles + lecture de gate concurrente ». Il ne ferme pas le parcours complet de la mission : les licences GitSpace/produit, l’AgentOS réel, le parcours LLM sans nouveau coût, les tests de refus cumulés et le benchmark conventionnel contrôlé restent ouverts. Le parcours LLM précédent reste `n=1`, et ne prouve aucun gain général.
