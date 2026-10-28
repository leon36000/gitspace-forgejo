# CONTINUATION-194 — Forgejo live 503, file receiver pleine et rejeu manuel

Date : 27 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié. Cette tranche ajoute un mode de scénario au runner Python; aucun code de production ni source Forgejo amont n’a été modifié.

## Lacune et références

CONTINUATION-193 prouvait le `503/server_busy` et la redélivraison exacte avec des clients loopback, mais n’exerçait ni l’émetteur Forgejo ni son rejeu natif. Le nouveau mode remplit la vraie file du receiver avant qu’une instance Forgejo locale émette une issue `opened`, observe le statut de livraison, puis reprend par la route manuelle de Forgejo et réconcilie les deux nouveaux UUID.

Forgejo source v17.0.0 est vérifié au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`; le clone source local est sale avec d’autres POC. Les cinq fichiers examinés (`services/webhook/deliver.go`, `models/webhook/hooktask.go`, `services/webhook/shared/payloader.go`, `services/webhook/webhook.go`, `routers/web/repo/setting/webhook.go`) sont inchangés par rapport à ce commit. Blobs, SHA-256 et sorties sont dans `forgejo-source-review.json`.

Dans le chemin `services/webhook/deliver.go::Deliver`, Forgejo marque le task livré avant l’appel HTTP et classe comme succès seulement un statut 2xx. `models/webhook/hooktask.go::FindUndeliveredHookTaskIDs` cherche les tâches dont `is_delivered=false`. Le chemin consulté ne reprogramme donc pas une tâche déjà tentée après un 503; cette lecture est bornée aux symboles ci-dessus. `models/webhook/hooktask.go::ReplayHookTask` recopie le payload et le type d’événement dans `CreateHookTask`, qui attribue une nouvelle UUID. `services/webhook/shared/payloader.go` place cette UUID dans `X-Forgejo-Delivery` et `X-Gitea-Delivery`.

## Parcours intégré

Commande finale :

```text
python3 scripts/run_forgejo_live_webhook_native_replay_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-webhook-503-native-replay-20261027-004 \
  --evidence-dir evidence/forgejo-webhook-503-native-replay-001/run-output-004 \
  --receiver-overload-first-delivery
```

L’instance temporaire Forgejo annonce `17.0.0+gitea-1.22.0`, Go 1.26.9; son exécutable `/tmp/forgejo-17.0.0` a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit de construction du binaire n’est pas établi. Le runner construit le receiver et le CLI Rust depuis le worktree courant dans son environnement local isolé.

Le scénario occupe les quatre workers avec quatre corps incomplets, remplit les seize places de file par des GET, puis crée l’issue dans Forgejo. La réponse qui remonte réellement du receiver à Forgejo est `503` avec `Retry-After: 5`; le task Forgejo #1 est `is_delivered=true`, `is_succeed=false`, statut 503, et son payload n’est pas ajouté au journal. Après le drain, les quatre connexions lentes reçoivent 408 et les seize probes 405. Une observation de six secondes ne voit aucun nouveau task ni POST automatique.

Deux POST manuels authentifiés sur `/forge-admin/agent-forge-core/settings/hooks/1/replay/{uuid}` créent deux tasks Forgejo distincts : les deux finissent en HTTP 202, chacun avec une nouvelle UUID et le même SHA du corps brut que l’essai 503. Le receiver journalise les deux reçus. Le reconciler traite leurs deux UUID dans un batch, relit une seule fois l’API issue avec le credential lecteur et renvoie deux états `current_state_read` identiques pour `forge-admin/agent-forge-core#1`. La réconciliation ne modifie pas l’inbox. Les reçus du run sont `run-output-004/result.json`, `hook-tasks.json`, `http-proxy.json`, `webhook-events.json`, `reconciliation.json`, `provenance.json` et leur `SHA256SUMS`.

## Vérifications et essais conservés

- `python3 -m py_compile scripts/run_forgejo_live_webhook_native_replay_e2e.py tests/test_forgejo_webhook_native_replay_e2e.py` — passe.
- `python3 -m unittest tests.test_forgejo_webhook_native_replay_e2e -q` — 30 tests réussis, dont le binding 503/Retry-After/file drain et ses refus.
- `git diff --check -- docs/architecture.md docs/checkpoint.md docs/source-inventory.md` — passe. Les nouveaux scripts/tests et la matrice étant non suivis par Git à ce HEAD, un scan explicite de trailing whitespace/NUL est conservé dans `verification/untracked-whitespace-check.log`.
- `python3 scripts/verify_cargo_license_state.py` — passe; confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`. `cargo metadata --offline --locked --all-features --format-version 1` passe aussi. Sorties sauvegardées sous `verification/`.
- `sha256sum -c SHA256SUMS` dans `run-output-004/` — les 14 reçus du scénario final passent.
- Le runner live ci-dessus — code 0, verdict `passed`; le readback final confirme l’arrêt des processus Forgejo, proxy et receiver ainsi que l’absence de leurs listeners.

Les trois essais rouges conservés et leurs codes de sortie sont explicites : `runner-output.log`/`run-output/result.json` s’arrête à `authenticated_native_replay` parce que le proxy n’a pas satisfait le binding attendu des tentatives; `runner-output-002.log` et `runner-output-003.log` s’arrêtent à `deactivate_hook_before_native_replay`, où la lecture SQLite n’a pas observé le statut attendu en 10 s. Après arrêt dans ces essais, le readback différé montrait le task inactif au statut 0. Le mode overload isole maintenant le critère 503/rejeu et omet la sous-épreuve indépendante de hook inactif, toujours exercée par le mode historique. L’essai final utilise un nouvel état jetable et passe. Les trois causes sont des limites/échecs du harnais ou de son délai d’observation, pas des preuves de réussite du scénario.

## Portée et limites

Preuve locale intégrée `n=1`, avec Forgejo v17 local, le binaire receiver Rust courant, un proxy HTTP loopback de mesure, la base SQLite jetable de Forgejo et le CLI de réconciliation courant. Les tâches Forgejo sont lues dans sa base locale en lecture seule; ce n’est pas une API stable. L’observation de six secondes ne suffit pas seule à prouver l’absence de toute reprise différée; le comportement du chemin audité est corroboré par `MarkTaskDelivered` et le filtre `FindUndeliveredHookTaskIDs`, sans audit exhaustif de chaque ordonnanceur Forgejo.

Le rejeu manuel crée une nouvelle livraison et son nouveau UUID n’est pas couvert par la déduplication exacte du receiver. Le reconciler ramène ici deux livraisons de la même issue à une seule lecture canonique; cette tranche ne démontre ni l’allocation idempotente d’une mission dans ce même run, ni un agent LLM/AgentOS, une PR, un merge, une charge soutenue, un SLO ou un gain de productivité. Le lien binaire↔commit de build et l’autorité de merge atomique restent ouverts. Pas de dépense fournisseur, publication, production, secret utilisateur ni code upstream copié/adopté.
