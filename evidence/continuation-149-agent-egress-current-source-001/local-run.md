# CONTINUATION-149 — contrôle egress au snapshot courant

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le runner a utilisé le binaire Forgejo local `/tmp/forgejo-17.0.0`, version annoncée `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit de construction de ce binaire n’est pas établi. Version Bubblewrap 0.9.0, Python 3.12.3. Les hashes précis des scripts et outils sont dans `source-sha256.txt`.

## Test exécuté

Commande :

```sh
python3 scripts/run_forgejo_agent_egress_live_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-egress-cont149 \
  --evidence-dir "$PWD/evidence/continuation-149-agent-egress-current-source-001/live-run"
```

Résultat : `verdict=passed`, trois sondes. Les contrôles hôte GET `/api/healthz`, GET dépôt API, GET dépôt Web répondent 200. La lecture hôte de `GET /api/v1/repos/forge-admin/mission-012-clamp/branches/main` avec credential `read:user,read:repository` répond 200; le credential n’est pas passé au sandbox. Le SHA avant/après est identique (`8188ed8381f927acf908c6535c0d769d08d6c6e9`).

Le sous-processus Bubblewrap s’exécute avec `--unshare-net --unshare-pid`, environnement nettoyé, `/tmp` et `/home` privés, et aucun credential monté. Il essaie les chemins directs API `GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/1`, Web `GET /forge-admin/mission-012-clamp/pulls/1` et Git `POST /forge-admin/mission-012-clamp.git/git-receive-pack` vers le vrai port local 3130. Les trois échouent par `ConnectionRefusedError` (errno 111) depuis le namespace `net:[4026535802]`, distinct du host `net:[4026531833]`. Le marqueur synthétique n’est pas visible; aucun état de branche ne change. Le serveur Forgejo se termine avec code 0 et le readback confirme l’absence du listener.

Reçu JSON : `live-run/result.json`; provenance : `live-run/provenance.json`; stdout/stderr : `live-run/driver.stdout.log` et `driver.stderr.log`; copie du probe : `live-run/sandbox-probe.py`.

## Portée et limites

Ceci est un canari n=1 du `local-test-adapter` courant, sans LLM/AgentOS. Il vérifie un contrôle hôte en lecture seule et trois routes précisément ciblées; il ne fait pas un merge, n’essaie pas de credential de fusion réel, ne couvre pas toutes les routes ni les acteurs privilégiés, le même UID, root, l’évasion de namespace ou le noyau. Le reçu MISSION-012 009 établit un parcours complet historique, mais la réconciliation CONTINUATION-140 montre une dérive de sources : ce canari ne remplace pas une validation actuelle du gate de fusion.

Le serveur jetable et sa base locale restent sous `/tmp/forgejo-egress-cont149` pour préserver l’état du test; les credentials créés par le harness sont synthétiques et locaux. Aucun fichier existant n’a été supprimé, aucune donnée n’a été envoyée à un service externe, aucun modèle ni production n’a été utilisé. Le canari Forgejo n’est pas adopté comme code produit. Projet toujours incomplet.

Suite : vérifier dans le code courant que l’autorité de fusion et ses credentials restent hors du processus agent, puis, seulement si cette liaison actuelle est ambiguë, bâtir un test ciblé de gate avec fixture locale; ne pas rejouer le parcours PR complet ni inférer une politique exclusive contre tous les writers.
