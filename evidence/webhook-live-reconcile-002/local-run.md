# WEBHOOK-LIVE-002 — parcours de réconciliation Forgejo local (préliminaire)

Date : 24 octobre 2026. Exécution sur le worktree local à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; ce commit est le parent Git observé, et les fichiers de travail étaient modifiés/non suivis. La provenance exacte des entrées exécutées est consignée dans [`provenance.json`](provenance.json), avec les empreintes SHA-256 de 19 fichiers source, l’état Git ciblé, le toolchain et les deux exécutables construits. Cette exécution est conservée comme reproduction fonctionnelle, mais elle est préliminaire pour l’isolation du build; voir [`WEBHOOK-LIVE-004`](../webhook-live-reconcile-004/local-run.md).

## Commandes

```sh
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_e2e.py
python3 scripts/run_forgejo_live_webhook_reconcile_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-reconcile-002-20261024-01 \
  --evidence-dir "$PWD/evidence/webhook-live-reconcile-002"
```

Le driver a construit `forgejo-webhook-local` et `forgejo-reconcile-local` par `cargo build --offline --locked --bin forgejo-webhook-local --bin forgejo-reconcile-local --target-dir /tmp/forgejo-live-webhook-reconcile-002-20261024-01/cargo-target`. Cargo n’a pas téléchargé de dépendances, mais le build s’est exécuté sur l’hôte avec `HOME`, `CARGO_HOME` et `RUSTUP_HOME` de l’utilisateur visibles; `--offline` ne bloque pas un script de build qui tenterait son propre accès réseau. Aucune fuite n’a été observée, mais ce run ne prouve pas l’isolation des scripts de build. Le binaire Forgejo testé rapporte `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le dépôt source officiel Forgejo inspecté séparément est un clone Codeberg de développement au commit `ea9ae1e174ac290febcd92f57e7a28ccb58262d0`; ce commit n’établit pas la provenance de construction exacte du binaire stable testé. L’inventaire note aussi la vérification GPG du binaire et ses limites de confiance locale dans [`source-inventory.md`](../../docs/source-inventory.md).

## Résultat observé

- Le serveur Forgejo jetable écoute sur `127.0.0.1:3130`; le dépôt `forge-admin/agent-forge-core` est privé.
- Le lecteur `forge-reader` est `is_admin=false`, `repo_permission_admin=false`, `repo_permission_push=false` et `repo_permission_pull=true`. Ses portées demandées sont `read:user,read:repository,read:issue`.
- Une issue privée ouverte (#1) a été créée dans Forgejo et lisible par le lecteur.
- Un événement `issues` correctement signé localement, avec le même identifiant de livraison deux fois, annonce un état périmé `closed`. Le vrai receiver accepte les deux requêtes en HTTP 202 et marque le deuxième reçu comme doublon sans ajouter une deuxième séquence.
- Le vrai CLI Rust relit l’issue par l’API Forgejo avec le credential de lecture transmis par pipe FD. Il renvoie `current_state_read`, un GET upstream, état canonique `open` et URL Forgejo attendue. L’issue est encore `open` au readback après l’exécution.
- Un credential invalide donne code de sortie 1, verdict `inconclusive`, erreur `upstream_unauthorized`, un seul essai et aucun `canonical_state`. Les octets du credential ne sont pas inclus dans stdout, stderr ni les artefacts persistés.
- L’instance Forgejo est arrêtée après le run; le contrôle de `127.0.0.1:3130` ne voit plus de listener.

Les résultats expurgés se trouvent dans [`result.json`](result.json), les lectures de rôle et de statut dans leurs fichiers dédiés. Les JSON générés sont en mode `0600`; le rapport ne contient aucun credential. `sha256sum -c SHA256SUMS` passe.

## Portée et limites

Le dépôt et l’API sont ceux d’une vraie instance Forgejo v17 locale; les exécutables receiver et réconciliation sont construits à partir du worktree puis lancés comme processus sous le même utilisateur. Aucun code de PR non fiable n’est exécuté. **Le webhook de ce run est synthétique** : Forgejo n’a pas été configuré comme émetteur. Le test couvre réception HMAC, déduplication et lecture réelle d’une issue privée, mais pas la livraison sortante dans cette exécution. Un run distinct antérieur `WEBHOOK-001` documente une livraison réellement émise par Forgejo avec tâche `IsSucceed=true` et HTTP 202; il est référencé dans [`evidence/local-run.md`](../../evidence/local-run.md) et vérifiable par son manifeste SHA-256. Aucun run ne démontre à lui seul l’intégration complète de ces deux parcours. Ce run n’exécute ni LLM, ni AgentOS, ni RAG, ni mémoire et ne mesure aucun gain de productivité ou de coût. Le rôle sans push a été confirmé par readback Forgejo; les portées OAuth sont celles demandées à la création, pas une introspection indépendante des claims du token.

Le répertoire d’état `/tmp/forgejo-live-webhook-reconcile-002-20261024-01` est conservé avec permissions privées (`0700` au répertoire, `0600` aux fichiers créés par le driver) pour inspection locale. Il contient la base SQLite, la configuration Forgejo et le secret HMAC temporaire; il n’est pas inclus dans les artefacts et ne doit pas être partagé. Le serveur et le receiver ont été arrêtés. Aucun nettoyage/suppression de cet état n’a été effectué.

Contrats correspondants : [`src/webhook.rs`](../../src/webhook.rs) vérifie HMAC et conserve le payload comme notification; [`src/webhook/http.rs`](../../src/webhook/http.rs) répond `reconcile_required` et idempotentise la livraison; [`src/bin/forgejo-webhook-local.rs`](../../src/bin/forgejo-webhook-local.rs) refuse les adresses non-loopback; [`src/bin/forgejo-reconcile-local.rs`](../../src/bin/forgejo-reconcile-local.rs) renvoie `current_state_read` seulement après lecture Forgejo et classe l’échec en `inconclusive`.

La première répétition de ce run est conservée séparément dans `webhook-live-reconcile-001`; son manifeste d’empreintes était partiel. `002` complète ces hashes, mais reste préliminaire sur l’isolation du build. `004` répète le parcours avec le build Bubblewrap hors réseau, sources/cache/sysroot en lecture seule, HOME hôte masqué et readback explicite des processus suivis et des ports TCP.
