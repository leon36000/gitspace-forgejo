# CONTINUATION-291 — revalidation du dépôt GitSpace officiel

Date : 27 octobre 2026. Worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit créé.

## Vérification

Exécuté : `git ls-remote https://github.com/leon36000/GitSpace.git refs/heads/main`. La sortie est `dcde1142cf76996a6ba0074cb7eab9d3c870c561 refs/heads/main`, identique au snapshot immuable déjà audité dans [CONTINUATION-168](../continuation-168-gitspace-upstream-source-audit-001/local-run.md) et consigné dans `docs/source-inventory.md`. Le commit ne change donc pas depuis le recontrôle du 26 octobre.

Le README public GitHub de `main` a également été consulté. Sa section Licence dit `UNKNOWN` et qu’aucune licence n’est choisie implicitement; le lien épinglé vers le commit vérifié est <https://github.com/leon36000/GitSpace/blob/dcde1142cf76996a6ba0074cb7eab9d3c870c561/README.md#L255-L257>. L’audit local borné de ce même commit a déjà inspecté 217 fichiers suivis, n’a trouvé aucun `LICENSE`, `LICENCE`, `COPYING` ou `NOTICE` et a vérifié que le dépôt contenait du code compilable en plus de documents. Le présent lot ne relance pas cette analyse complète et ne copie aucun fichier.

## Limites et conséquence

`git ls-remote` établit l’OID retourné pour `main` pendant cette commande, non la permanence future de la branche. La page web est une source publique rendue/cachée et ne remplace pas une lecture du tree courant; l’égalité avec le commit audité rend toutefois le snapshot de source déjà acquis pertinent pour cette vérification. Une licence `UNKNOWN` et l’absence de fichier de licence ne déterminent pas les droits de réutilisation ou de redistribution. Ceux-ci restent à clarifier directement avec l’ayant droit; aucun code GitSpace n’est adopté, copié ou distribué.

Aucun test logiciel, clone, service, appel modèle ou dépense n’a été lancé dans ce lot. Les versions, commande, reçu `git ls-remote` et empreintes sont joints.
