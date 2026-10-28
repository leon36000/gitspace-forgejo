# WEBHOOK-LIVE-EMITTED-014 — issue modifiée et réconciliation Forgejo réelle (24 octobre 2026)

## Reproduction

Le run utilise le worktree local modifié au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Forgejo est la version `17.0.0+gitea-1.22.0`; SHA-256 du binaire `/tmp/forgejo-17.0.0` : `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. L’origine exacte de compilation du binaire n’est pas établie. Les hashes des sources inspectées, exécutables et outils sont dans [`provenance.json`](provenance.json).

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v
python3 scripts/verify_cargo_license_state.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.test_cargo_license_state -v
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-014-state \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-014"
(cd evidence/webhook-live-emitted-014 && sha256sum -c SHA256SUMS)
```

Le build des deux exécutables Rust s’est fait avec Cargo/Rust 1.95.0, hors réseau, sous Bubblewrap 0.9.0, registre, sources et sysroot en lecture seule, `--offline --locked`; la commande et ses contraintes sont consignées dans [`cargo-build.log`](cargo-build.log). Le répertoire privé du run est mode `0700`; les secrets de hook et API n’apparaissent pas dans les reçus. Les chemins d’état et de preuve ne sont pas réutilisables: le runner refuse de les remplacer.

## Résultat observé

Les 16 tests du runner Python passent. Forgejo a émis six livraisons réellement reçues par le receiver, chacune liée à une tâche SQLite Forgejo distincte avec HTTP 202 et UUID concordant : issue `opened`, issue `closed`, deux issues `edited`, PR `opened`, PR `closed`. Les deux événements `edited` ont des identifiants et séquences différents. Après fermeture, un seul PATCH modifiant titre et corps a produit deux notifications; le premier essai WEBHOOK-LIVE-EMITTED-013 a échoué car le runner attendait à tort une seule notification. Cet échec et son nettoyage sont conservés dans le dossier 013; le run 014 prend explicitement en charge les deux événements.

La réconciliation relit l’état courant par API Forgejo pour chacun des événements d’issue, y compris les notifications anciennes `opened` et `edited`; le verdict est `current_state_read` et l’état est `closed`. Les deux événements PR se réconcilient aussi sur la PR fermée et non fusionnée, avec les mêmes SHAs base/tête. Le lecteur API a `pull=true`, `push=false`, `admin=false`. Le credential invalide est refusé en verdict `inconclusive` (`upstream_unauthorized`), sans état canonique. L’API confirme le hook désactivé; les processus sortent et les deux listeners sont absents.

Les reçus détaillés sont [`result.json`](result.json), [`webhook-events.json`](webhook-events.json), [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json), les readbacks de réconciliation et [`port-cleanup-readback.json`](port-cleanup-readback.json). Les tests du garde Cargo sont 5/5; l’état reste `license=null`, `license_file=null`, `publish=[]`, licence non choisie et droits de redistribution non évalués. Les logs de vérification et sommes sont dans ce répertoire.

## Portée et limites

Ce run vérifie l’émission et la réconciliation des notifications Forgejo, pas un parcours LLM. Aucun Claude, AgentOS, Forge Code, RAG ou service mémoire n’a été appelé; aucun gain de qualité, coût ou délai n’est mesuré. Le code candidat n’est pas exécuté. Forgejo, receiver et CLI partagent l’UID hôte; le sandbox Bubblewrap ne couvre que le build. Le build exact du binaire Forgejo reste inconnu. Le test démontre un comportement sur une exécution locale, pas une garantie générale de livraison ou d’ordre.

Les états privés des runs 013/014 dans `/tmp` sont préservés; ils contiennent les secrets jetables locaux et ne doivent pas être supprimés sans décision explicite.
