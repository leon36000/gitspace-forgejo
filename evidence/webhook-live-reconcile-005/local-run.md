# WEBHOOK-LIVE-005 — réconciliation Forgejo locale avec build isolé

Date : 24 octobre 2026 (EDT). Run local sur le worktree dont le parent Git observé est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les sources Rust et les drivers étaient modifiés ou non suivis. `provenance.json` lie les hashes des sources, du registre Cargo mis en scène, du toolchain, de Bubblewrap et des deux exécutables.

## Reproduction

```sh
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_e2e.py
git diff --check -- scripts/run_forgejo_live_webhook_reconcile_e2e.py
python3 scripts/verify_cargo_license_state.py
python3 scripts/run_forgejo_live_webhook_reconcile_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-reconcile-005-20261024-01 \
  --evidence-dir "$PWD/evidence/webhook-live-reconcile-005"
```

Le driver a compilé `forgejo-webhook-local` et `forgejo-reconcile-local` avec Rust/Cargo 1.95.0 et Bubblewrap 0.9.0. Le build utilise un namespace réseau et PID séparé, `--clearenv`, `/home` remplacé par un tmpfs, HOME redirigé vers `/tmp`, et les sources, le cache crates.io vérifié par `Cargo.lock` et le sysroot montés en lecture seule. Cargo a reçu `--offline --locked`; les seules sorties exportées sont les deux binaires explicitement nommés. Le journal et la politique exacte sont dans [`cargo-build.log`](cargo-build.log) et [`provenance.json`](provenance.json).

## Résultat observé

- L’instance Forgejo `17.0.0+gitea-1.22.0` (SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`) a servi une issue privée ouverte dans `forge-admin/agent-forge-core`.
- Forgejo confirme `forge-reader` non administrateur, avec permissions dépôt `admin=false`, `push=false`, `pull=true`. Les scopes du token sont ceux demandés à sa création; leurs claims n’ont pas été introspectés indépendamment.
- Un webhook signé synthétique annonce à tort `closed`; le vrai receiver répond HTTP 202, puis répond `duplicate=true` au rejeu du même delivery ID sans nouvelle séquence. **Dans ce run, Forgejo n’est pas l’émetteur du webhook.**
- Le vrai CLI Rust utilise le token de lecture transmis par pipe FD pour relire l’API Forgejo : verdict `current_state_read`, un GET upstream, état canonique `open`; l’issue reste ouverte au readback. Un token invalide produit exit 1, `inconclusive/upstream_unauthorized`, un essai et aucun `canonical_state`.
- Les deux processus suivis terminent avec code 0. Le readback `ss -ltnp` voit les ports TCP 3130 et 54289 libres après leur arrêt ([`port-cleanup-readback.json`](port-cleanup-readback.json)). Cela atteste ces PID et ces listeners attendus pour ce run, pas l’absence universelle de descendants ou d’autres ressources.
- `sha256sum -c SHA256SUMS` passe après ajout de ce rapport; tous les artefacts du dossier, manifeste compris, sont en mode `0600`.

Une preuve distincte d’émission sortante Forgejo existe dans WEBHOOK-001 : le hook local actif a livré une issue, Forgejo a enregistré `IsSucceed=true` et HTTP 202, et le readback du hook confirme son URL loopback. Ses artefacts sont décrits dans [`evidence/local-run.md`](../../local-run.md), [`webhook-001-reproduction.md`](../../webhook-001-reproduction.md), [`webhook-001-live-run.json`](../../webhook-001-live-run.json) et [`webhook-001-hook-readback.json`](../../webhook-001-hook-readback.json); leurs empreintes dans `evidence/SHA256SUMS` ont été vérifiées séparément. Ces runs complémentaires ne forment pas un unique run bout-en-bout.

## Portée et limites

Ce run exerce un vrai service Forgejo v17 local et les deux binaires Rust de ce worktree. Le build est isolé du HOME utilisateur et du réseau sous Bubblewrap. Forgejo, le receiver et le CLI de réconciliation s’exécutent toutefois sous le même utilisateur hôte; aucun candidat de PR hostile ni isolation OS multi-tenant n’a été testé. Aucun LLM, AgentOS, RAG ou service mémoire n’a été utilisé, et aucune amélioration générale de succès, coût ou durée n’est mesurée.

Le webhook de ce run est synthétique; la livraison Forgejo→receiver a été exercée séparément dans WEBHOOK-001, pas dans ce scénario de lecture d’état. Le binaire Forgejo utilisé rapporte v17.0.0; les sources officielles taguées `v17.0.0` ont été inspectées séparément, sans preuve que ce binaire local ait été construit à partir du clone source précis. Les scripts documentent ce lien de provenance comme inconnu.

Le répertoire `/tmp/forgejo-live-webhook-reconcile-005-20261024-01` est conservé en mode privé. Il contient la base, la configuration Forgejo et le secret HMAC du test; il est exclu des preuves persistées et n’a pas été supprimé.
