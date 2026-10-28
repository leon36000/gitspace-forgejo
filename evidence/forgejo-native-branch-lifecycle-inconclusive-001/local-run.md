# FORGEJO-NATIVE-BRANCH-LIFECYCLE-INCONCLUSIVE-001

Date : 26 octobre 2026. Worktree local à HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, checkout fortement modifié. Pas de commit.

## Lacune et changement du harnais

Les preuves antérieures couvraient deux moitiés séparées : Forgejo natif create → delete quand la branche est recréée avant réconciliation, et le refus d’un delete isolé dont le GET renvoie 404. Le script refusait explicitement de combiner --include-create et --expect-branch-absent-after-delete.

Le validateur de lot accepte maintenant deux résultats attendus selon l’état connu par le harnais :
- branche courante présente : deux résultats current_state_read liés au même SHA et une lecture upstream;
- branche supprimée avant réconciliation : code 1 / lot inconclusive, deux résultats ordonnés resource_not_found non réessayables, chacun lié au bon delivery et à la ressource, sans canonical_state, avec un message indiquant que le 404 ne distingue pas l’absence d’un accès masqué.

Le validateur refuse un succès canonique quand l’état attendu est absent et refuse un 404 dont l’action ne préserve pas cette ambiguïté. Les nouveaux tests unitaires vérifient également les IDs/order, le cache à un seul read et le chemin précédent où la branche existe.

## Parcours avec Forgejo local

Le script démarre Forgejo local v17.0.0 sur 127.0.0.1:3130, crée un dépôt privé jetable et branche agent/lifecycle/inconclusive-001, puis un client Git local pousse la branche. Forgejo émet un vrai webhook Gitea create; le runner constate le SHA de création et avance la branche pour rendre ce SHA obsolète. Il supprime ensuite la branche. git ls-remote --heads confirme que le ref n’est plus publié. Avant toute recréation, le runner demande au vrai CLI Rust de réconcilier les deux livraisons natives.

Les deux tâches Forgejo ont reçu HTTP 202. Le batch retourne code 1 et verdict=inconclusive; chaque delivery, create puis delete, reçoit resource_not_found, non réessayable, sans état canonique. Le CLI rapporte une lecture upstream pour les deux IDs du même ref; le test du code négatif cache confirme séparément la politique de batch et le run cible rapporte upstream_read_attempts=1. La boîte d’entrée reste inchangée. Le reader local a seulement pull=true, push=false, admin=false; le credential reste fourni via pipe FD et n’est pas enregistré. Le hook est désactivé et relu, le serveur et le receiver s’arrêtent, et leurs listeners observés sont absents.

C’est un résultat positif du harnais parce que le refus inconclusif correspond à l’oracle de sécurité. Il ne certifie pas que la branche est canonique « absente » : le 404 demeure ambigu par contrat, malgré la confirmation Git séparée que la ref est supprimée. Le compteur réseau vient du CLI, pas d’une capture indépendante.

## Faux libellé du reçu

Le JSON brut live-run/result.json dit par erreur qu’aucun service live n’a été contacté. Le run a effectivement lancé la seule instance Forgejo locale; le même résultat enregistre sa version, le listener loopback, deux livraisons natives réussies et l’arrêt du processus. Le reçu brut est préservé. Le fichier result-correction.json explique le libellé erroné et consigne la version corrigée dans le runner pour les prochains runs. Le hash source du runner au moment de l’exécution est conservé dans la provenance brute; le changement après le run ne modifie que cette phrase descriptive. Aucun run n’est réécrit.

## Vérifications et provenance

La classe BranchRefBatchReconciliationValidationTests passe 8/8; le module tests.test_forgejo_emitted_webhook_e2e passe 106/106. AST et compilation Python passent. Le builder compile les binaires Rust en ligne déconnectée sous Bubblewrap 0.9.0, à partir de Cargo.lock, registry et toolchain read-only; les hashes de build et les logs sont dans live-run/. Forgejo indique 17.0.0+gitea-1.22.0, Go 1.26.9, SHA-256 e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7. La source amont auditée est Forgejo v17.0.0 au commit 0d791421f85ca13a63b665053d81ea765fbbc571; le commit de build du binaire n’est pas établi.

Le contrôle local confirme Cargo license=null, license_file=null, publish=[], no_license_selected, droits not_assessed, SPDX NOASSERTION. Aucun changement de licence ou lockfile. Les commandes, versions, résultats et empreintes courants/au moment du run sont dans commands.txt, toolchain.txt, verification-results.json et source-hashes.txt.

## Portée

Instance Forgejo locale jetable, une branche et un run (n=1). Le scénario démontre la réception native de deux événements de cycle de vie et le refus batch fail-closed après suppression. Il ne teste ni modèle/AgentOS, création de PR, vérification indépendante, autorité de fusion, comportement post-fusion, writers concurrents ni comparaison de productivité. Aucun service externe, production, donnée privée, coût ou code Forgejo/GitSpace n’est concerné. Le produit reste sans licence choisie.


Le contrôle additionnel verify_live_run_receipt.py revalide le lien create/delete, les deux IDs ordonnés, le 404 ambigu, l’absence de canonical_state, les droits minimaux observés, le journal inchangé, le cleanup et les huit empreintes brutes du sous-dossier live-run. Il vérifie également que la correction de libellé référence le hash source du run et que le JSON original reste conservé. Résultat : toutes les assertions passent.
