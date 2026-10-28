# POC-064 — ref déplacée avant la confirmation post-receive LFS

Date : 25 octobre 2026. Scratch : /tmp/forgejo-v17-base-cas-poc, basé sur Forgejo officiel v17.0.0 au commit 0d791421f85ca13a63b665053d81ea765fbbc571. Le worktree contient les modifications expérimentales des POC antérieurs; il ne représente pas un checkout upstream intact. Cette tranche ajoute un test au fichier d’intégration déjà modifié; aucun fichier de production n’est changé par POC-064.

## Observation testée

Le test TestPOCGenericFileLFSRecoveryDefersUntilTargetRefReachesCandidate active le serveur LFS d’intégration, crée le fichier .gitattributes requis, puis écrit un fichier suivi par LFS via le chemin réel d’update de fichiers et push. Le test seam de post-receive déplace la ref de branche vers le commit de base, après que le push candidat est accepté mais avant la lecture de ref utilisée pour promouvoir les métadonnées. Le hook constate que la ref cible ne pointe plus vers le candidat et laisse l’association pending.

L’association n’est pas active et le GET LFS renvoie 404. Une première récupération manuelle ne promeut rien et compte une opération deferred. Après que le test rétablit la ref exacte vers le commit candidat, une nouvelle récupération promeut l’association et le GET LFS renvoie 200 avec les octets attendus. Cela traverse l’API fichiers, pre/post-receive, stockage/métadonnées LFS et le helper de récupération dans le serveur d’intégration SQLite local.

Commande : GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPOCGenericFileLFSRecoveryDefersUntilTargetRefReachesCandidate$'. Résultat : 1/1, code 0. Le log final contient aussi des erreurs de logger au teardown du harness (CreateNotice avec contexte annulé) et pour le GET 404 attendu; l’assertion et la suite du test terminent PASS. Le log et le code sont conservés ici. Source hashes et versions sont joints.

## Interprétation et limites

Cette interleaving montre le côté conservateur avant la vérification post-receive : pending reste invisible tant que le commit n’est pas atteignable depuis la ref exacte, puis la reprise manuelle le promeut quand elle l’est. POC-060 reproduit l’autre fenêtre, où la récupération observe un ref contenant le candidat, celui-ci est déplacé avant la transaction de promotion DB, et l’association est tout de même activée. Les deux cas établissent que la vérification Git puis la mutation DB ne sont pas atomiques; la promotion dépend de l’ordre de la course. Ils ne prouvent pas à eux seuls une divulgation entre dépôts et ne choisissent pas si la sémantique doit refléter l’état courant ou une publication acceptée au moins une fois.

Le déplacement est injecté par un test seam appelant update-ref; ce n’est pas une concurrence entre deux clients distants. La récupération est appelée par le test, pas par un worker de production. Le test n’établit ni la récupération après crash à chaque instruction, ni migration/backfill, fairness, rétention/nettoyage, dialectes autres que SQLite, ni l’ordre de démarrage du serveur. Aucun code Forgejo n’est adopté ou copié au produit; aucun LLM, service externe ou environnement de production n’est utilisé.
