# CONTINUATION-315 — vérification statique du contrôle de merge Forgejo

Date : 27 octobre 2026. Dépôt produit HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dirty. Lot documentaire de revue de code amont; aucun code Forgejo/Rust/Python n’a été modifié pour ce lot, aucun build ni test Go n’a été lancé, aucun serveur additionnel n’a été démarré et aucun coût externe n’a été engagé.

## Conclusion circonscrite

Dans Forgejo officiel v17.0.0, la route API normale `POST /repos/{owner}/{repo}/pulls/{index}/merge` avec `Do=merge` appelle `CheckPullMergeable`, puis la méthode de fusion. Le check vérifie la protection de branche et le nombre d’avis; la méthode de merge reçoit le SHA de tête attendu de la PR. Ce chemin permet donc de tester qu’un merge normal a passé le check de protection juste avant la fusion.

Il ne crée pas à lui seul un reçu atomique de la décision d’approbation. Le check transactionnel rend son résultat, puis l’API appelle le merge séparément. Le code examiné pour `Merge` et `createTemporaryRepoForMerge` compare la tête attendue de la PR mais n’expose pas de condition sur la version de la liste des revues ni un SHA de base attendu. Aucune validation atomique des mêmes avis dans la transaction de push n’a été trouvée dans ces extraits. Une révocation concurrente entre ces étapes est donc une **hypothèse de course déduite de l’ordre des appels**, pas une course reproduite ni une preuve d’exploitation.

Le compteur de protection s’appuie sur les flags persistés `official` et `dismissed`; il ajoute le filtre `stale=false` uniquement si `IgnoreStaleApprovals` est activé. `IsOfficialReviewer` décide à l’écriture si le compte passe la whitelist de reviewers. Le gate local applique en plus sa propre allowlist d’IDs; il ne doit pas déléguer cette autorisation au seul nombre d’approbations Forgejo.

La route manuelle a un cas `MergeCheckTypeManually` qui retourne avant la vérification de la protection. Le mode auto emprunte une voie différée distincte; son contrôle complet au moment de l’exécution n’a pas été suivi dans ce lot. Le drapeau `force_merge` peut également changer le comportement pour un administrateur; il était absent du POST de la fixture CONT-314. Ce reçu ne conclut donc que sur le chemin API normal utilisé en laboratoire.

## Observations liées

La tentative CONT-314 `...-002` avait la règle relue `dismiss_stale_approvals=true`, `apply_to_admins=true`, `required_approvals=1` et une allowlist de reviewer. Le runner est arrivé à l’étape `admit_emitted_issue_delivery` puis a échoué avec `mission_requirement_arbitration_insufficient`; l’ordre du runner place cette étape après la fusion et l’émission/réception du webhook. Son rapport de refus est [failure.json](../continuation-314-forgejo-v3-source-pr-admission-002/failure.json) et sa règle est [branch-protection-readback.json](../continuation-314-forgejo-v3-source-pr-admission-002/branch-protection-readback.json). CONT-314-005 contient le succès avec `dismiss_stale_approvals=false` et l’avis post-fusion `stale=true,dismissed=false`. Ces runs illustrent l’invalidation après fusion; ils ne démontrent pas le résultat d’une révocation concurrente dans la fenêtre.

## Couverture et prochaine expérience

Inspection à HEAD `0d791421f85ca13a63b665053d81ea765fbbc571` avec `git show HEAD:<path>`; hashes, fichiers et symboles précis dans [source-review.txt](source-review.txt). Le fichier `services/pull/merge.go` du scratch complet est modifié par des POC antérieurs; les observations de ce lot proviennent de la version Git au HEAD, pas du fichier de travail modifié. Ne pas copier ou adopter le patch déjà présent.

Prochaine expérience : une barrière test-only dans le scratch autour de l’intervalle check→merge, avec une seconde identité qui tente de révoquer l’avis ou de modifier la protection. Tester que le merge échoue ou enregistrer exactement le cas où il passe. Éviter le faux positif : emprunter le chemin API général, laisser `force_merge` absent, désactiver le mode auto et ne pas appeler la route « manuel ». Inspecter aussi le contrôle post-receive avant de proposer un reçu hôte. Aucun résultat comparatif ou décision de fork n’est établi ici.
