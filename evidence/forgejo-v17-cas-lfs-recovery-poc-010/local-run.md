# FORGEJO-CAS-LFS-RECOVERY-POC-010 — reprise du staging après installation du ref

## Source et périmètre

- Dépôt officiel Forgejo `v17.0.0`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`, scratch isolé `/tmp/forgejo-v17-base-cas-poc`.
- Environnement : Go `1.27.3 linux/arm64`, Git `2.43.0`, SQLite. Le patch reste dans le scratch; aucun code Forgejo n’est copié au dépôt produit.
- Ce POC prolonge le patch expérimental CAS/LFS de POC-008. L’arbre comprend 14 fichiers suivis modifiés (`+499/-30`) et deux fichiers nouveaux de récupération (191 lignes), soit 16 fichiers touchés dans ce scratch. Le hash du diff suivi est `6e5d9676289c4d9df5ce2e570b8978c3ae1453fe698839aca710aa4d539cd83e`; les SHA-256 des 16 fichiers sont dans `scratch-touched-files.sha256`.

## Protocole essayé

`services/pull/lfs_recovery.go::RecoverLFSPendingMetaObjects` parcourt les stages d’un dépôt et valide chaque SHA candidat comme identifiant Git. Il exécute `git for-each-ref --contains=<candidate>` dans le dépôt cible. Si au moins un ref atteint le commit candidat, il promeut de façon idempotente toutes les associations de l’opération dans une transaction. Si aucun ref ne l’atteint, il laisse le stage intact. Si le commit est absent, le dépôt est illisible ou la promotion échoue, il conserve le stage et retourne l’erreur. La récupération ne supprime jamais les stages.

Le test `TestRecoverLFSPendingMetaObjectsAfterInterruptedPostReceive` utilise une vraie base Forgejo de test et un dépôt Git bare temporaire. Il prépare une association, place le commit dans le dépôt puis supprime le ref temporaire : le passage de récupération diffère la promotion et garde l’association invisible. Le test avance ensuite directement le ref de branche sans appeler le hook post-receive, comme scénario d’interruption après la mise à jour Git. La récupération suivante publie l’association; une répétition ne modifie plus rien.

`TestRecoverLFSPendingMetaObjectsDefersMissingCandidate` confirme qu’un SHA absent ne vaut pas acceptation : l’erreur est rapportée, aucune association active n’est créée et la ligne pending reste présente.

La stratégie est monotone : un candidat injoignable ne déclenche pas de suppression automatique. Cela évite de perdre un stage pendant un push lent ou quand Git est momentanément indisponible, au prix de stages et objets LFS potentiellement retenus sans limite.

## Vérifications

Les deux suites suivantes ont été relancées sans cache; leurs sorties sont conservées dans ce dossier :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/pull
```

Résultat : `ok forgejo.org/services/pull 0.695s`.

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./models/git
```

Résultat : `ok forgejo.org/models/git 0.336s`. `git diff --check` passe. Les empreintes des deux sorties et de la liste des sources sont consignées dans `SHA256SUMS`.

## Limites et décision

Ce résultat teste le modèle et la fonction de service dans le scratch; il ne lance pas un worker Forgejo et le code de récupération n’est appelé nulle part automatiquement. Le saut du hook est simulé par une mise à jour directe d’un dépôt bare, pas par l’arrêt d’un processus Forgejo vivant. Le test ne transfère pas de contenu avec `git-lfs`, ne contrôle pas les droits privés, ne teste pas les appels simultanés ni la course avec une suppression concurrente de refs, et n’intègre pas tous les writers/API.

La recherche porte sur tous les refs du dépôt. Un candidat retenu par n’importe quel ref est promu; un candidat qui ne devient jamais joignable reste pending et peut retenir le contenu jusqu’à une politique séparée de rétention/nettoyage. Ce POC fournit une règle de reprise fail-closed, mais il ne fournit pas encore de worker périodique, de métriques, d’alerte, de TTL ou d’opération de nettoyage examinée.

**Décision :** conserver le travail comme expérience et ne pas adopter le patch/fork. La prochaine vérification Forgejo devrait traiter le planificateur/worker, les writers qui créent des métadonnées avant ref, l’absence de récupération destructive, les doublons/concurrence, les dépôts privés et la rétention. Pour le pilote produit actuel, l’intégration par interfaces garde la priorité sous réserve de prouver l’autorité exclusive de merge.
