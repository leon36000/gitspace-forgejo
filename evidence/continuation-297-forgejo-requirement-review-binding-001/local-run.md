# CONTINUATION-297 — faisabilité de liaison des exigences à une revue Forgejo

Date : 27 octobre 2026. Audit de source ciblé dans le worktree produit fondé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` et le scratch Forgejo `/tmp/forgejo-v17-base-cas-poc`. Le scratch est globalement modifié par des POC, mais les treize fichiers officiels ciblés sont identiques à `HEAD`. Le tag exact est `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les empreintes SHA-256 des blobs Git officiels et leurs Git blob IDs sont dans [`forgejo-v17-source-sha256.txt`](forgejo-v17-source-sha256.txt).

Aucun service ou processus n’a été lancé, aucune API n’a été appelée, aucun credential n’a été lu, aucun test n’a été exécuté et aucun code Forgejo n’a été copié.

## Ce que le serveur expose

Dans `routers/api/v1/api.go:1020–1062`, les routes PR incluent `GET /pulls/{index}` et `GET /pulls/{index}/reviews`; leur groupe est protégé par `mustAllowPulls()` et `reqRepoReader(unit.TypeCode)`. Un lecteur du dépôt peut donc obtenir ces observations sans privilège administrateur. `routers/api/v1/repo/pull.go::GetPullRequest` renvoie un DTO dont `modules/structs/pull.go` expose `merged`, `merge_commit_sha`, `base` et `head`.

`routers/api/v1/repo/pull_review.go::ListPullReviews` charge la liste et son compte, applique les options page/limite et fournit `X-Total-Count`. `modules/structs/pull_review.go::PullReview` expose `id`, `user`, `state`, `commit_id`, `official`, `stale`, `dismissed` et le lien de la PR. `services/convert/pull_review.go::ToPullReview` conserve ces valeurs. Le contrat suffit à filtrer un instantané sur la portée PR, un compte autorisé, un état approuvé, un head exact, `official=true`, `stale=false` et `dismissed=false`, si le lecteur récupère toutes les pages et traite les changements observés comme inconclusifs.

Cette lecture reste paginée et non transactionnelle : PR, pages de revues, contenu au commit de fusion et branche courante peuvent changer entre GET. `commit_id` lie l’avis au head de la PR; Forgejo n’expose pas dans ce DTO le commit de base que le reviewer aurait vu. Une seconde lecture peut détecter certaines dérives, sans fournir un snapshot atomique ni supprimer le TOCTOU.

## Limites d’identification de la PR source et du reviewer

`GET /repos/{owner}/{repo}/commits/{sha}/pull` (`routers/api/v1/repo/commits.go::GetCommitPullRequest`) appelle `models/issues.GetPullRequestByMergedCommit`, qui recherche uniquement `merged_commit_id`. Cette route ne retrouve pas une PR générale à partir du commit qui a modifié un fichier. Un numéro de PR stocké dans une future exigence doit rester un pointeur non fiable : l’adaptateur vérifierait scope, dépôt de base, branche, état fusionné, SHA de fusion et égalité exacte des octets du document au commit fusionné et à la branche protégée courante.

Le modèle interne définit `User.IsBot()` dans `models/user/user.go`, mais le DTO `modules/structs/user.go::User` ne contient pas `is_bot`, et `services/convert/user.go::toUser` ne le projette pas. Les réponses publiques fournissent l’ID du compte; une allowlist hôte prouve seulement que cet ID est autorisé par la politique configurée. Elle ne prouve pas que le compte est une personne ni qu’il n’est pas un compte de service.

## État de l’adaptateur produit et décision

`src/lib.rs::Requirements` (v2) ne contient ni historique de décisions, ni justification, ni PR source. `src/mission_admission.rs::MissionAdmissionSourceV1` lit issue, protection, fichier aux octets exacts et inventaire du dépôt; `prepare_admission` refuse les ambiguïtés explicites, mais n’authentifie pas les révisions non ambiguës. Le port dispose d’une implémentation locale HTTP dans `src/bin/forgejo-mission-local.rs::LocalLoopbackForgejoSource`, qui lit issue, branche protégée, exigences et arborescence; cette implémentation ne lit pas les PR ou leurs revues. Les chemins et empreintes de ces sources courantes sont dans [`source-sha256.txt`](source-sha256.txt).

Architecture retenue pour la prochaine tranche, sans changement du noyau Forgejo : une future version de Requirements portera un journal structuré des résolutions et justifications ainsi qu’un numéro de PR source, jamais un `reviewer_id` auto-déclaré. Une politique d’admission hôte, inaccessible au workspace agent, portera la portée Forgejo, les IDs de comptes autorisés et le seuil de revue. L’adaptateur relira toutes les pages et liera les octets exacts à `merge_commit_sha` et au head protégé courant; toute absence, page incomplète/dérivante, PR hors scope/non fusionnée, contenu différent, revue périmée/rejetée/non officielle ou compte non autorisé refusera l’allocation. Il faudra annoncer que le snapshot reste non atomique et que la règle autorise des comptes Forgejo, pas des personnes vérifiées.

Ce rapport établit la faisabilité des champs et routes v17 ciblés, non le comportement d’une instance live, la preuve qu’un utilisateur est humain, une politique hôte déjà choisie, ou l’implémentation du nouvel adaptateur. Aucun test, schéma, binaire ou comportement de produit ne change dans cette tranche. Le détail des sources officielles est dans les commandes et empreintes jointes.
