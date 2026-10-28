# FORGEJO-NATIVE-DELETED-RESOURCE-RECONCILE-001

Date : 26 octobre 2026, vers 21:09 UTC d'après les timestamps de l'instance locale. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD lu : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était déjà fortement modifié. Aucun reset, nettoyage, commit, publication, changement de production ou appel de modèle n'a eu lieu.

## Scénario et résultat

Une instance locale jetable de Forgejo `17.0.0+gitea-1.22.0` a émis les webhooks natifs `issues/opened` pour deux ressources distinctes. Les tâches Forgejo et les entrées JSONL du récepteur sont reliées par UUID et SHA-256 du corps; les deux livraisons ont reçu HTTP 202. L'issue #3 a ensuite été supprimée par l'identité propriétaire (HTTP 204), et son absence a été relue par cette même identité (HTTP 404). Un lecteur API non administrateur a encore lu l'issue sœur #2.

Le vrai `forgejo-reconcile-local` a reçu les deux IDs dans l'ordre #3 puis #2. L'observateur HTTP indépendant a vu exactement deux GET authentifiés avec le credential lecteur : `/issues/3` → 404, puis `/issues/2` → 200. Le premier résultat est `inconclusive/resource_not_found`, avec l'avertissement correct qu'un 404 ne distingue pas une ressource absente d'un accès refusé. Le résultat sœur est `current_state_read` pour l'issue ouverte #2. Le code de sortie du CLI est 1, le verdict du lot est `inconclusive`, et le journal inbox reste identique octet pour octet. Ainsi, le 404 d'une ressource ne masque pas la lecture réussie d'une ressource sœur.

Le verdict `passed` dans `result.json` signifie que les assertions de ce scénario ciblé sont satisfaites; il ne transforme pas le verdict métier du lot (`inconclusive`) en succès. `api-observation-proxy.json` porte `native_deleted_resource_validated=true` et aussi `validated=false` : ce dernier champ correspond à l'observation générale utilisée par les autres modes, qui n'a pas été exécutée dans ce mode ciblé. Il ne s'agit pas d'une validation globale de toute la suite.

## Reproduction et vérifications

La commande E2E exécutée était :

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -B scripts/run_forgejo_live_webhook_native_replay_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace --state-dir /tmp/forgejo-native-deleted-resource-reconcile-20261026-001 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/forgejo-native-deleted-resource-reconcile-001 --native-deleted-resource-only
```

Le processus s'est terminé avec le code 0; le runner n'a pas écrit de résumé sur stdout. Le résultat et les preuves structurées sont dans ce dossier. La suite Python ciblée, relancée sans bytecode pour archiver sa sortie, passe 27/27 (`focused-tests.log`). `git diff --check` passe. `cargo metadata --format-version 1 --no-deps` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed` et `NOASSERTION`; aucune licence produit n'est choisie.

La construction hors ligne/verrouillée sous Bubblewrap utilise `--unshare-net`, `--unshare-pid`, environnement nettoyé, sources/registre/sysroot en lecture seule. Versions et empreintes : `build-provenance.json`, `provenance.json`, `cargo-build.log` et `toolchain.txt`. Le binaire Forgejo a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le dépôt officiel v17.0.0 examiné est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, mais le commit qui a construit ce binaire n'est pas établi. Les sources de runner/test utilisées sont liées par leurs SHA-256 dans `provenance.json`; le checkout est dirty.

## Portée et limites

C'est une répétition `n=1` d'un cas d'intégration Forgejo local, avec deux issues et deux livraisons. Elle démontre le traitement indépendant d'un 404 et d'un 200 dans un batch et le refus honnête de conclure sur le 404 du lecteur. Elle ne démontre pas la recréation de la ressource ni la reprise ultérieure des mêmes IDs, la concurrence, la fusion, un LLM/AgentOS, ni un gain comparatif. Les tests de contrat du runner ne remplacent pas d'autres tests de refus. La présence des reçus et du code de sortie ne prouve pas une propriété de sécurité générale. Aucun code Forgejo ou GitSpace n'a été copié.

Le cleanup enregistré indique l'arrêt des processus/listeners, désactivation du webhook avant l'arrêt du serveur, et `cleanup_failure=null`. L'instance est locale; aucun service externe ni secret n'a été utilisé.
