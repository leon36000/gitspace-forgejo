# CONTINUATION-204-A1 — échec du premier raccord live

Le test-first Rust a d'abord retourné zéro test à cause de `--exact` sans nom qualifié; ce retour code 0 a été écarté comme preuve. La commande corrigée a exécuté le nouveau test et échoué sur la cible par défaut `forge-admin` au lieu de `forge-team-org`, avant l'implémentation.

Le premier lancement du harnais live a compilé avec Bubblewrap les deux binaires Rust sous `--offline --locked`, puis a échoué avant le démarrage de Forgejo : `write_private_json` appelait `write_safe_json` sur le module d'helpers Forgejo qui ne l'expose pas. La correction a routé la sérialisation vers le module fixture qui possède déjà ce garde. Ce run n'a créé ni webhook ni token; aucun serveur n'avait démarré. Son dossier d'état privé `/tmp/forgejo-team-review-cont204-state-001` et son `cargo-build.log` restent préservés.

Le run corrigé est CONSIGNÉ séparément sous [CONTINUATION-204-A2](../continuation-204-live-org-team-reviewer-rust-reconcile-002/local-run.md). Ce dossier conserve les sorties observées de l'échec initial; il n'est pas utilisé comme preuve de réussite.
