# CONTINUATION-054 — séparer un credential d’écriture réel du processus MCP

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Dépôt fortement modifié; aucun reset ni commit.

## Commande et identité

```text
python3 scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-agent-egress-stdio-mcp-007 --evidence-dir "$PWD/evidence/forgejo-agent-egress-stdio-mcp-007"
```

Instance locale Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; Linux `7.0.0-1019-nvidia aarch64`, Bubblewrap 0.9.0, Python 3.12.3 et Cargo/rustc 1.95.0. Les sources du runner, du wrapper, de l’adaptateur, du MCP fixture, des helpers et du test sont snapshotées sous `source/`; versions et empreintes sont dans `provenance.json`.

## Résultats

- Les routes hôtes répondent : health/API dépôt/Web dépôt 200; la route PR API et la page PR sont 404 parce que le dépôt fixture n’a pas de PR #1; le `git-receive-pack` sans authentification répond 401. L’instance active est donc distinguée d’un serveur absent.
- Le même processus `mission_012_test_agent_adapter.py --stdio-mcp` lancé sous `bwrap --unshare-net --unshare-pid` vise `127.0.0.1:3130`. Il classe les probes API, Web et `git-receive-pack` comme bloquées, rapporte `network_connected=false`, un namespace réseau et un namespace PID distincts, les seuls descripteurs `0,1,2` et les seules clés d’environnement autorisées (`HOME`, `LANG`, `LC_ALL`, `PATH`, `PWD`, `PYTHONDONTWRITEBYTECODE`). Le superviseur n’observe aucune connexion sur son listener synthétique.
- La sonde du child tente de lire `/proc/<pid-hôte>/fd/<fd>` pendant que ce descripteur contient un vrai token Forgejo local à portée `read:user,read:issue,read:repository,write:issue,write:repository`. La lecture échoue dans le namespace PID du child. Ce token reste en mémoire dans le runner et dans un `memfd` marqué `CLOEXEC`; le runner vérifie qu’il n’est pas sérialisé dans la configuration MCP, la commande ou l’environnement. `close_fds=True` est utilisé au lancement; le child observe uniquement `0,1,2`.
- À la barrière `tools/call:mission_context`, le processus de tâche demeure vivant. Le gate de confiance utilise le token pour lire `main` dans Forgejo (HTTP 200) pendant l’attente; le SHA est identique avant, pendant et après. Le gate libère ensuite l’adaptateur et celui-ci continue ses appels MCP.
- La tâche Rust fixture échoue réellement avant correction (`cargo test --offline --locked`, exit 101, attendu 10, obtenu -2), modifie `src/lib.rs` à travers `workspace_write_text`, puis réussit l’acceptation (exit 0). Les sorties Cargo, sources initiale/finale et checksums sont conservés.
- `tests.test_mission_012_sandboxed_test_adapter` passe 7/7; la suite Python complète passe 239/239 en 19,892 s. `py_compile` des quatre scripts concernés et `git diff --check` passent. Le runner confirme l’arrêt propre de son instance Forgejo (code 0, listener absent après run).

## Ce que la preuve établit et ses limites

Elle exerce la frontière processus/réseau du vrai mode `--stdio-mcp` contre les routes d’une instance Forgejo active et vérifie que le processus de tâche ne reçoit pas le token d’écriture que le gate emploie pendant l’attente MCP. Le secret n’est pas conservé dans les sorties; le token Forgejo reste dans le seul état jetable sous `/tmp` sous forme de hash côté base et de contenu volatile en mémoire/memfd pendant le runner.

Le credential a des scopes d’écriture, mais ce lot n’appelle pas `POST /merge` et ne prouve pas une autorisation de fusion pour cette PR. Il n’y a pas de PR dans ce dépôt fixture. La preuve précédente [FORGEJO-AGENT-EGRESS-MERGE-001](../forgejo-agent-egress-merge-001/local-run.md) utilise une autorité de fusion réelle pendant un probe-only; la combinaison exacte d’un child `--stdio-mcp` avec un POST de fusion reste à faire si nécessaire.

Le moteur est `local-test-adapter`, pas un LLM ou AgentOS. Le MCP host est une fixture locale allowlistée. Forgejo et le runner partagent le même UID; aucune résistance multi-tenant, noyau compromis ou évasion n’est établie. Le résultat vise les trois routes testées et cet hôte; il ne garantit pas une absence universelle d’egress. Il ne s’agit pas d’un parcours PR/merge de bout en bout ni d’un benchmark comparatif. Aucun service externe, coût de modèle ou système de production n’a été utilisé.
