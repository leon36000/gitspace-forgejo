# CONTINUATION-263 — receiver attesté → réconciliation contre Forgejo v17 local

Date : 2026-10-27. Worktree : `.runtime/webhook-005-workspace`; HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. La révision était fortement modifiée et le run a compilé cet arbre tel quel; le HEAD ne décrit pas ces changements locaux.

## Parcours exécuté

Le runner a construit `forgejo-webhook-local` et `forgejo-reconcile-local` avec Cargo `--offline --locked` dans Bubblewrap (`--unshare-net --unshare-pid`, environnement vidé, sources/registre/sysroot en lecture seule). Il a démarré le binaire Forgejo local `/tmp/forgejo-17.0.0`, version rapportée `17.0.0+gitea-1.22.0`, sous `127.0.0.1:3130`, puis créé un dépôt privé et un lecteur `forge-reader` non administrateur, autorisé en lecture seule (pull=true, push=false, admin=false).

Le runner a créé une graine receiver Ed25519 dans l’état privé, dérivé la clé publique depuis le binaire receiver, puis envoyé au receiver réel une livraison HMAC synthétique `issues/closed` qui ne venait pas de Forgejo. Le receiver répond 202 au premier envoi (`duplicate=false`, séquence 1), puis 202 à la rediffusion identique (`duplicate=true`, même séquence). Le CLI construit depuis le worktree a reçu la clé de confiance et un credential via FD; il a relu l’API Forgejo et rendu `current_state_read` (schéma 12), avec l’état actuel `open` et une seule lecture, bien que le payload synthétique disait `closed`. Le contenu d’issue reste explicitement `untrusted_user_content`.

Le même runner a essayé un credential invalide : sortie non nulle et `inconclusive/upstream_unauthorized`, sans état canonique et avec `credential_material_recorded=false`. Les lectures de run/job Actions inexistants retournent aussi `inconclusive`/404; Actions était désactivé et aucun runner n’a tourné. Résultat global : `passed`. La preuve indique que les deux processus suivis ont terminé et que les ports Forgejo/receiver étaient absents après arrêt.

## Versions, intégrité et vérification

Le binaire Forgejo rapporte `17.0.0+gitea-1.22.0` et son SHA-256 est `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. La provenance enregistre `rustc 1.95.0`, Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, hash des deux exécutables de l’adaptateur, hash de l’arbre source et hash des 22 fichiers source. `sha256sum -c SHA256SUMS` passe pour tous les artefacts produits par le runner. Les sources et les manifestes append-only de cette note ajoutent leurs propres empreintes tout en conservant le préfixe initial de ce dossier et les préfixes globaux.

Le répertoire d’état du run reste dans `/tmp/forgejo-attestation-263-state`, mode 0700, pour préserver les données du run; aucun nettoyage par suppression n’a été fait. Les réponses et la provenance conservées ne contiennent pas le credential brut.

## Portée et suite

C’est un parcours isolé réel receiver → journal signé → CLI → lecture d’issue Forgejo v17, avec doublon et refus d’authentification; ce n’est pas le parcours complet de mission jusqu’à PR/fusion. Le webhook est synthétique et n’établit pas la livraison native Forgejo→receiver. L’exécution de build est isolée, mais les processus runtime ont tourné sous l’utilisateur courant, sans preuve d’une frontière OS contre un agent hostile. Aucun AgentOS, LLM, runner Actions, test de correction logicielle ou comparaison contrôlée n’a été exécuté. Le numéro de version et le hash du binaire sont consignés; ce run n’établit pas le commit source exact dont ce binaire Forgejo a été construit. Un seul run n’établit ni un gain ni un résultat général.
