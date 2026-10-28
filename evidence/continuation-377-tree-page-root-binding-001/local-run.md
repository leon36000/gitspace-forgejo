# CONTINUATION-377 — relier les pages d’arbre d’exigences au même SHA

Date : 28 octobre 2026. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Lacune et correction

L’adaptateur Forgejo pagine `/git/trees/{commit}` pour inventorier les exigences. Le contrôle précédent vérifiait notamment le numéro de page, le compte total et les chemins, mais n’assurait pas que toutes les pages décrivaient le même arbre. Deux réponses pouvaient donc garder `total_count` constant tout en venant de deux snapshots différents.

Le parseur exige maintenant le champ `sha` de la réponse, sous une forme de révision Git valide. `list_requirement_paths_at` mémorise le premier SHA d’arbre et refuse `InvalidResponse` dès qu’une page suivante annonce un SHA différent. Le refus `TreeTooLarge` reste prioritaire pour les réponses qui dépassent la limite, même si elles ne contiennent pas de SHA.

Le test `tests::requirement_tree_pagination_rejects_pages_from_different_tree_roots` sert deux pages ayant le même total, mais des SHA différents; l’inventaire est refusé. Les fixtures positives de pagination portent le même SHA. La suite confirme aussi pagination complète/ordonnée, rejet des liens symboliques et classification de l’arbre trop grand.

## Vérification

- Le groupe ciblé `tests::requirement_tree` passe 4/4.
- `cargo test --offline --locked --bin forgejo-mission-local` passe 48/48; sortie conservée dans `forgejo-mission-local-tests.log`.
- `cargo fmt --all -- --check` et `git diff --check` passent; sorties conservées dans leurs fichiers respectifs.
- Les commandes et résultats sont récapitulés dans `verification-commands.txt`; versions d’outils, HEAD et empreinte SHA-256 du binaire source local dans `environment.json`.

Le contrat de réponse a été vérifié dans Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, `modules/structs/repo_tree.go::GitTreeResponse.SHA`. Blob : `86b221e1feb0c110c66b1f4df40e3c33aa54b8fa`; SHA-256 du fichier de cet objet : `afcf0270d4b6325b29674089c096696503c4ca91e0ad93ce2fa4c53042264ed0`. Le fichier dans le scratch n’est pas modifié. L’extrait et les commandes de vérification sont dans `source-review.txt`.

## Portée et limites

Ce lot est une preuve de contrat avec fixtures HTTP loopback et tests du binaire Rust; aucune instance Forgejo n’a répondu pendant ce test. L’égalité de SHA entre pages détecte une pagination mélangée, mais ne prouve pas que le serveur renvoie des chemins honnêtes correspondant au commit, ne rend pas la lecture atomique et n’empêche pas une dérive suivie d’un retour entre lectures. Aucun code Forgejo ou GitSpace n’est copié. Aucun LLM, AgentOS, PR, merge ou gain comparatif n’est démontré.

La demande de transfert GitLab reste bloquée séparément : le remote `pc4-gitlab` vise le dépôt communiqué, mais une requête HTTPS anonyme vers PC4 échoue actuellement par résolution DNS (`curl: (6) Could not resolve host`, HTTP `000`). Aucun credential n’a été envoyé, aucune référence distante n’a été lue, et aucun commit/push n’a eu lieu. Le jeton retrouvé par la recherche de courriel vise d’autres projets et reste inutilisé; il faut une route PC6→PC4 fonctionnelle et un credential renouvelé, limité à `root/gitspace-forgejo`. Le probe et sa sortie sont consignés dans `connectivity-preflight.txt`.

La mission globale demeure incomplète. Prochaine tranche locale : lier à une source hôte protégée le digest d’exigences et la commande de vérification, comme relevé après CONTINUATION-343; ne pas traiter la vérification de pagination de ce lot comme une preuve d’autorité des critères.
