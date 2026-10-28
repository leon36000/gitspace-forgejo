# FORGEJO-NATIVE-PUSH-OUT-OF-SCOPE-007

Date : 25 octobre 2026. Exécution locale dans une instance Forgejo jetable. Le worktree source était à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Question et correction

Le reconciler refusait déjà un événement dont `owner/repository` ne correspond pas à son profil, avant de lire le credential ou d’appeler Forgejo. Il codait toutefois ce garde local comme `upstream_forbidden`, alors qu’aucune permission Forgejo n’avait été vérifiée. Cela donnait à l’appelant une action incorrecte.

Le test rouge `red-test-output.log` établit l’écart : le code renvoyé était `upstream_forbidden`, alors que le contrat du garde doit produire `resource_out_of_scope`. Le code dans `src/bin/forgejo-reconcile-local.rs::classify_out_of_scope_delivery_error` ne reclassifie qu’un refus `upstream_forbidden` lié à un dépôt différent du profil; le chemin d’un vrai 403 sur le dépôt configuré reste `upstream_forbidden`. Les tests Rust couvrent les événements hors périmètre en appel unique et en batch, le batch mixte, ainsi que la conservation de la classification 403 sur le dépôt configuré.

## Parcours Forgejo local

Commande exécutée :

```text
python3 scripts/run_forgejo_native_push_webhook_reconcile.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-push-out-of-scope-007-run2 --evidence-dir evidence/forgejo-native-push-out-of-scope-007-run2
```

Forgejo `17.0.0+gitea-1.22.0`, Go `1.26.9`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. La même instance locale a hébergé deux dépôts privés : `forge-admin/agent-forge-core` (cible du profil) et `forge-admin/native-push-out-of-scope-proof` (hors périmètre). Deux vrais pushes Git HTTP dans chaque dépôt ont produit quatre livraisons `push` Forgejo et quatre tâches réussies, HTTP 202. La tête de la branche étrangère a été relue par l’API Forgejo.

Le batch normal du dépôt cible garde son résultat `current_state_read`, traite l’événement ancien et l’événement courant avec une lecture upstream, consomme le credential fourni par pipe et ne modifie pas l’inbox. Pour le batch des deux livraisons étrangères, le CLI sort avec le statut 1 et retourne `inconclusive`, deux erreurs `resource_out_of_scope`, `retryable=false`, chacune liée au bon UUID et nommant le dépôt configuré et le dépôt de l’événement. `upstream_read_attempts=0`, le pipe conserve le credential complet et l’inbox reste identique octet par octet. Aucun contenu de credential n’est conservé dans les artefacts.

Le runner a vérifié la désactivation des deux hooks par relecture API. Forgejo et le receiver se sont arrêtés avec succès; aucune adresse d’écoute restante n’a été observée. Les détails expurgés sont dans `result.json`, `foreign-push-events.json`, `foreign-hook-tasks.json` et `out-of-scope-refusal.json`.

## Vérifications

- `cargo test --locked --offline --all-targets --quiet` : 511 réussis, 0 échec, 61 ignorés, 34 groupes. Les tests ignorés n’ont pas été exécutés.
- `python3 -m unittest discover -s tests -p 'test_*.py'` : 160 réussis.
- `python3 -m unittest -v scripts/test_cargo_license_state.py` : 7/7.
- `cargo clippy --locked --offline --all-targets --quiet` : code de sortie 0; des avertissements Clippy restent présents.
- `cargo fmt --all -- --check`, `git diff --check` et `py_compile` passent.
- Readback Cargo : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`.

Versions locales : Python 3.12.3, Git 2.43.0, Cargo/rustc 1.95.0. Les logs de commandes et les manifestes sont dans ce dossier.

## Couverture et limites

C’est une preuve isolée de routage/refus contre Forgejo local v17.0.0, avec de vrais webhooks `push` natifs et le CLI réel. `upstream_read_attempts` est le compteur du CLI; ce parcours ne capture pas indépendamment les paquets du batch étranger. Le test de conservation du credential lit les octets restants dans le pipe. Il n’exerce ni LLM/AgentOS, ni PR/fusion, ni comparaison de productivité, ni charge concurrente. Il ne démontre rien sur un déploiement multi-tenant.

Un premier essai est conservé dans `../forgejo-native-push-out-of-scope-007/result.json`. Il s’est arrêté pendant le provisionnement du deuxième dépôt avec un `RuntimeError`; le runner n’a enregistré que le type de l’erreur, donc sa cause exacte n’est pas établie. Le run `007-run2` a réussi en créant les deux dépôts avec le token bootstrap autorisé. Le premier dossier et son résultat sont préservés.

Aucun code Forgejo/GitSpace n’a été copié ou modifié dans le produit; aucune publication, modification de production ou demande externe n’a eu lieu. Licence du produit toujours non choisie; droits GitSpace toujours indéterminés. Le projet reste incomplet.
