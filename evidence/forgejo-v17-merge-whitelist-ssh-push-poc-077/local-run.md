# FORGEJO-V17-MERGE-WHITELIST-SSH-PUSH-POC-077

Date : 26 octobre 2026. Référence Forgejo v17.0.0 : commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. L’intégration a été exécutée dans le checkout scratch séparé et modifié `/tmp/forgejo-v17-base-cas-poc`; aucun code Forgejo n’a été copié dans le produit.

## Observation

`TestPOCMergeWhitelistActorCannotSSHDirectPushToProtectedBranch` crée un collaborateur non-admin ayant l’écriture sur le dépôt, puis protège la branche par défaut avec `EnablePush=false` et une whitelist de merge contenant ce même acteur. Le readback API rapporte `UserCanMerge=true` et `UserCanPush=false`. Ce compte fusionne une PR par l’API normale. Le test inscrit ensuite une clé SSH générée pour ce compte, clone le dépôt avec le chemin SSH du propriétaire, crée un commit local et tente un push SSH direct sur la branche protégée. Le hook `pre-receive` refuse le push; le readback du dépôt confirme que le SHA de base n’a pas changé.

Le scratch comporte un hook expected-base expérimental. Le test désactive cette option avant la sonde SSH, tout en laissant la protection de branche activée, afin que le refus mesuré soit celui de la permission de branche. La diff par rapport au HEAD officiel ajoute la vérification expected-base et son helper, sans modifier le bloc de contrôle de permission de branche qui suit. `cmd/serv.go` et `routers/private/serv.go` sont identiques octet pour octet à leurs objets officiels.

Le premier essai a échoué dans le harnais : l’URL SSH ciblait le namespace de l’acteur alors que le dépôt appartient à un autre compte. Le chemin a été corrigé pour utiliser celui du propriétaire; le run ciblé final passe. L’échec initial venait du montage du test, pas de Forgejo.

## Reproduction et résultat

La commande exacte, les versions, les identifiants des objets sources officiels, les empreintes, le résumé de sortie assaini et le hash du log brut du scratch figurent dans les fichiers adjacents. Commande finale :

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPOCMergeWhitelistActorCannotSSHDirectPushToProtectedBranch$'
```

Résultat : un test d’intégration ciblé passe, code de sortie 0. Serveur, base de données, clé générée et opération SSH restent dans le harnais local Forgejo. Aucun service distant ni instance de production n’a été contacté.

## Sources et interprétation

- À l’objet officiel, `cmd/serv.go::allowedCommands/runServ` autorise `git-receive-pack` comme écriture et délègue le contrôle d’accès dépôt/clé.
- `routers/private/serv.go::ServCommand` résout le dépôt et la clé publique, détermine l’utilisateur authentifié et vérifie le mode d’accès demandé.
- `routers/private/hook_pre_receive.go::preReceiveBranch` vérifie le droit d’écrire sur la branche puis applique les règles de protection et de merge. La sonde exerce ce chemin SSH local en désactivant la politique expected-base expérimentale.
- Les identifiants Git d’objets officiels et SHA-256 figurent dans `source-objects.txt` et `source-sha256.txt`; le test scratch et son log brut ont aussi leur propre SHA-256.

Cette observation de compatibilité/sécurité est étroite : un compte non-admin, un dépôt, une branche protégée par défaut, SQLite et le serveur d’intégration Forgejo en processus local. Elle n’est ni un audit exhaustif ni un test d’OpenSSH externe, des tags, deploy keys, autres refs, writers concurrents, PostgreSQL, cluster ou configuration de production. Elle n’établit pas une autorité de merge unique. POC-076 montre toujours qu’un collaborateur merge-whitelisted peut renommer une branche protégée par l’API; le refus SSH ne ferme donc pas l’autre chemin d’écriture. Aucun fork n’est adopté.

## Relation au client de merge du produit

La revue en lecture seule de [`merge_authority_request_is_allowed`](../../src/bin/forgejo-reconcile-local.rs:1204) relève une liste positive côté hôte. Pour le client de merge, elle n’autorise que des lectures du dépôt configuré : `/pulls/{n}`, revues paginées canoniquement, branche `main`, issue, commits et trees; s’y ajoutent `/user` et `/users/{login-sûr}`. La seule écriture est un POST normal de merge avec `Do="merge"` et un SHA complet du candidat. Le client distinct `MergePolicyReadOnly` est seul autorisé à lire `branch_protections`. Les tests existants `merge_authority_policy_allows_only_gate_reads_and_one_merge_route` et `merge_authority_policy_rejects_other_writes_before_network_or_credential_read` couvrent notamment le refus de PATCH/DELETE de branche; `expected_profile_gate_requests` compare la séquence d’appels émise. Cette partie est une revue du code existant, pas un nouveau run de produit dans ce POC.

La politique client s’exécute avant la lecture du credential hôte ou l’ouverture réseau, mais ne réduit pas les droits côté serveur Forgejo et ne protège pas un token utilisé hors de ce processus hôte de confiance. Le numéro de PR est validé par sa forme puis doit rester lié à la mission, au candidat et à la preuve dans le flux appelant. L’agent ne doit recevoir ni token, ni FD, ni capacité d’appeler ce processus authentifié.

## Artefacts de vérification

- `test-summary.log` est filtré et assaini depuis le log brut du scratch; son hash est dans `source-sha256.txt`.
- `SHA256SUMS` couvre le rapport et les entrées conservées. `manifest-check.txt` enregistre le résultat local. `verify_global_manifest.py` vérifie les hashes courants de ce lot et des documents canoniques dans l’index append-only; résultat dans `global-manifest-check.txt`.
- Le worktree produit reste fortement modifié. Ce lot actualise seulement architecture, inventaire, checkpoint et preuves; il ne modifie ni le code Forgejo du produit ni l’état de production.
