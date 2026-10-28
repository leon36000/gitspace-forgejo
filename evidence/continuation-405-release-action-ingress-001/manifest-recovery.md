# Recovery du manifeste CONTINUATION-405

Le premier append du manifeste a ouvert `manifest-append.json` en redirection avant l’exécution du générateur. Le générateur a donc correctement haché les octets présents à ce moment-là (fichier vide), puis la redirection a écrit le résumé JSON après le calcul. Le contrôle local a détecté l’écart avant de créer les reçus de validation.

Correction append-only : conserver toutes les lignes existantes, ajouter le hash courant de `manifest-append.json` et du présent reçu au manifeste local, puis ajouter ces hashes, le nouvel hash du manifeste local et le nouvel hash de l’index worktree aux deux index globaux. Les validations traitent les occurrences par chemin en privilégiant la dernière entrée, sans supprimer ni réécrire les anciennes lignes. Le préfixe worktree et projet est comparé au SHA-256 enregistré avant le premier append.

Aucune donnée utilisateur ou credential n’est dans ce correctif. Ce reçu documente uniquement l’intégrité des fichiers de preuve de ce lot.
