# Vérification de la forme du webhook `issue_comment`

Révision du code officiel Forgejo v17.0.0 dans le scratch local : `0d791421f85ca13a63b665053d81ea765fbbc571` (`/tmp/forgejo-v17-base-cas-poc`).

Dans `modules/structs/hook.go::IssueCommentPayload`, `PullRequest` est un champ racine sérialisé en `pull_request`; `Issue` est un autre champ racine, et `IsPull` est sérialisé en `is_pull`. `services/webhook/notifier.go::CreateIssueComment` renseigne `PullRequest` pour une PR et laisse ce pointeur vide pour une issue ordinaire. Les chemins d’édition/suppression définissent également la référence PR avant de construire le payload. Le routage actuel par présence d’un objet racine `pull_request` correspond donc au code source vérifié. Les fixtures de processus représentent ce schéma; aucune modification produit ni test supplémentaire n’était nécessaire.

Les empreintes SHA-256 des deux fichiers upstream sont `42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12` (`modules/structs/hook.go`) et `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f` (`services/webhook/notifier.go`). Le HEAD du scratch correspond au commit v17.0.0 ci-dessus. Ces fichiers n’ont pas été copiés dans le produit; aucun comportement live n’a été rejoué par cette revue.
