# WEBHOOK-LIVE-007 — émission Forgejo réelle et readback de hook corrigé

Date : 24 octobre 2026 (EDT). Run isolé sur le worktree dont le parent Git observé est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; des changements suivis et non suivis préexistants sont présents. [`provenance.json`](provenance.json) lie les hashes des sources Rust, des quatre scripts de build/run et du test Python, ainsi que le registre Cargo, Bubblewrap, le toolchain et les binaires. Le binaire Forgejo v17 local est identifié par hash; sa provenance exacte depuis le clone source officiel tagué `v17.0.0` au commit `0d791421f85ca13a63b665053d81ea765fbbc571` n’est pas établie.

## Reproduction

```sh
python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py
python3 scripts/verify_cargo_license_state.py
git diff --check
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-007-20261024-03 \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-007"
```

Les six tests Python passent, y compris le test ajouté après la revue indépendante : `active_after_run` n’est `false` que lorsque le GET Forgejo confirme la désactivation; si la désactivation ou le readback manque, la valeur est `null`. Le test a échoué avant le correctif puis passe : voir le [log rouge](emitted-007-hook-readback-red.log) et les [six tests verts](emitted-007-unit-tests.log). `py_compile`, le garde de licence et `git diff --check` passent; leurs sorties sont conservées ici. Le build du receiver et du CLI utilise Bubblewrap avec namespaces réseau/PID, HOME masqué, environnement nettoyé, sources/registre/sysroot montés en lecture seule, Cargo offline/locked et limites de ressources. La politique et les hashes sont dans [`provenance.json`](provenance.json) et [`cargo-build.log`](cargo-build.log).

## Résultat observé

- Forgejo `17.0.0+gitea-1.22.0` (binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`) écoute seulement sur `127.0.0.1:3130`. Une issue privée #1 du dépôt `forge-admin/agent-forge-core` passe de `open` à `closed`.
- Le hook Forgejo émet deux livraisons réelles `issues` vers le receiver Rust (`opened`, puis `closed`). Les deux tâches Forgejo sont marquées livrées/réussies, HTTP 202; leurs UUID correspondent en ordre aux `delivery_id` du journal.
- Le CLI relit l’API actuelle pour l’ancien et le nouveau delivery : `current_state_read`, une lecture upstream, état canonique `closed`. Le credential invalide donne exit 1 et `inconclusive/upstream_unauthorized`, sans état canonique.
- `forge-reader` est confirmé non admin, sans permission push et avec lecture dépôt. [`hook-readback.json`](hook-readback.json) rapporte `active_after_run=false` et `deactivation_readback_confirmed=true`; le readback de nettoyage concorde. Les PIDs Forgejo et receiver terminent avec code 0 et les ports TCP attendus sont absents.
- Le manifeste SHA-256 couvre le rapport, les readbacks, les résultats, les journaux et la provenance; dossier `0700`, fichiers `0600`.

## Revue et correction du run précédent

La revue read-only de WEBHOOK-LIVE-006 a trouvé que son `hook-readback.json` sérialisait `hook_disabled` directement comme `active_after_run`, inversant le sens. Les autres éléments de 006 (`result.json`, le readback API dans le driver et le nettoyage) indiquent que le hook était désactivé; l’artefact seul était erroné et ne suggérait pas une fuite réelle. Les reçus runtime originaux de 006 sont inchangés; son manifeste couvre maintenant également la note d’erratum. Le test rouge/vert de sérialisation et le nouveau run 007 corrigent et remplacent cette preuve. La revue a aussi signalé l’omission du source de test dans la provenance 006; 007 l’inclut.

## Portée et limites

Le parcours intégré couvre l’émission Forgejo, la réception HMAC, le journal Rust, la réconciliation de deux notifications vers l’état courant et le refus de credential invalide. Il ne couvre pas dans cette même exécution demande→exigence→mission→modification→PR→vérification indépendante→fusion. Le readback des tâches dépend de la table SQLite interne `hook_task` de Forgejo v17, pas d’une API publique produit. Les processus de l’instance et du receiver partagent l’UID hôte; aucun code candidat hostile n’est exécuté. Les doublons/désordres, reprises après crash et changements concurrents ne sont pas exercés par ce run précis.

Aucun LLM, AgentOS, RAG ou service mémoire n’a été utilisé; aucune comparaison de coût, temps ou taux de succès n’est établie. L’allowlist `127.0.0.1` n’appartient qu’à cette instance jetable. Les répertoires privés `/tmp/forgejo-live-webhook-emitted-006-20261024-01`, `...-02` et `/tmp/forgejo-live-webhook-emitted-007-20261024-03` sont conservés sans suppression; le dernier contient la base et les secrets temporaires, exclus des preuves persistées.
