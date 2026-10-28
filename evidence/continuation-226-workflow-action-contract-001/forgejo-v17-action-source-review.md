# Source Forgejo v17 — valeurs d’action workflow

- Dépôt officiel : `https://github.com/forgejo/forgejo`.
- Commit examiné : `0d791421f85ca13a63b665053d81ea765fbbc571`.
- Chemin local du scratch : `/tmp/forgejo-v17-base-cas-poc`.
- `modules/structs/hook.go:521-523` : `new_run_attempt`, `run_status_changed`, `run_completed`.
- `modules/structs/hook.go:543-545` : `new_job_attempt`, `job_status_changed`, `job_completed`.
- SHA-256 du fichier : `42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12`.
- `git diff --quiet HEAD -- modules/structs/hook.go` : code 0; aucune modification locale de ce fichier.

Le scratch est globalement dirty (POC de concurrence/LFS). Cet audit lit uniquement le fichier proprement rattaché au commit officiel; aucun code Forgejo n’est copié, modifié ou exécuté pour ce lot. Ce contrôle confirme les constantes du type Go, pas le comportement des émissions webhook en exécution ni toutes les versions.
