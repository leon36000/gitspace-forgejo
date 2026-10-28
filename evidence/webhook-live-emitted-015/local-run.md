# WEBHOOK-LIVE-EMITTED-015 — liaison des résultats au delivery demandé (24 octobre 2026)

## Reproduction

Le run part du worktree local modifié, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Forgejo annonce `17.0.0+gitea-1.22.0`; SHA-256 du binaire `/tmp/forgejo-17.0.0` : `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le build exact du binaire n’est pas établi. La provenance complète, y compris les hashes des sources et des exécutables Rust, est dans [`provenance.json`](provenance.json).

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v
python3 scripts/verify_cargo_license_state.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.test_cargo_license_state -v
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-015-state \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-015"
(cd evidence/webhook-live-emitted-015 && sha256sum -c SHA256SUMS)
```

Le build Rust emploie Cargo/Rust 1.95.0 sous Bubblewrap 0.9.0, avec réseau et namespace PID retirés, environnement nettoyé, sources/registre/sysroot en lecture seule et `--offline --locked`; la commande est consignée dans [`cargo-build.log`](cargo-build.log). État privé mode `0700`; les secrets jetables ne figurent pas dans les reçus. Les chemins de cet essai ne doivent pas être réutilisés: le runner refuse d’écraser ses dossiers.

## Résultat et correction

Une revue statique a constaté que le runner passait le bon `delivery_id` au CLI, mais ne comparait pas cet ID au résultat reçu. Le test négatif a d’abord échoué faute de validateur; `validate_reconciliation_delivery_binding` exige maintenant l’égalité et est appliqué aux résultats des événements d’issue, PR et credential invalide. Le test direct refuse un résultat d’un autre ID ou sans ID.

Le run Forgejo v17 passe après cette correction. Six tâches émises par Forgejo sont confirmées reçues en HTTP 202 et liées par UUID aux événements journalisés: issue `opened`, `closed`, deux `edited`, PR `opened`, `closed`. Les deux `edited` ont des IDs distincts et des séquences 3 et 4. Les six réconciliations réussies renvoient l’ID exact demandé; le résultat au credential invalide renvoie lui aussi le bon ID tout en restant `inconclusive/upstream_unauthorized`. Les états canoniques lus par l’API Forgejo sont fermés pour l’issue et la PR non fusionnée. Un readback séparé des reçus confirme **7/7** bindings, dont le refus.

Les 17 tests Python ciblés passent; le garde de licence passe 5/5 et confirme `license=null`, `license_file=null`, `publish=[]`, licence non choisie, redistribution `not_assessed`. Détails : [`result.json`](result.json), [`webhook-events.json`](webhook-events.json), [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json), [`result-binding-readback.log`](result-binding-readback.log) et [`port-cleanup-readback.json`](port-cleanup-readback.json). Le hook est relu inactif; les processus sortent et les ports écoutés sont absents.

## Portée et limites

Cette tranche renforce le runner de preuve; elle ne modifie pas le runtime Rust. Elle observe une instance Forgejo locale avec n=1, sans modèle, AgentOS, Forge Code, RAG ni mémoire; aucun gain de qualité, coût ou délai n’est mesuré. Le code candidat n’est pas exécuté; Forgejo et receiver partagent l’UID hôte. Bubblewrap isole le build uniquement. Le binaire Forgejo n’a pas de provenance de compilation exacte établie. Cela ne garantit pas l’ordre des livraisons en production.

Les états privés des runs 013–015 dans `/tmp` sont conservés et ne doivent pas être supprimés sans décision explicite.
