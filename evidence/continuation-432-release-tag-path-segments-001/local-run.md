# CONTINUATION-432 — compatibilité des URLs de tags Release avec Forgejo v17

Date : 2026-10-28. Exécution locale et hors ligne dans le worktree fortement modifié local/WEBHOOK-005-canonical-links, HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Aucun changement de branche, commit, push, écriture distante ou accès à une instance de production dans ce lot.

## Défaut et correction

L’audit du code Forgejo officiel v17.0.0, commit 0d791421f85ca13a63b665053d81ea765fbbc571, établit le format canonique : modules/util/url.go:12-20 partage le tag sur /, applique url.PathEscape à chaque segment et rejoint les segments sans échapper le séparateur; models/repo/release.go:184-187, Release.HTMLURL, construit /releases/tag/ avec cette fonction; services/convert/release.go:20-23 expose r.HTMLURL() dans la réponse API. Dans le scratch Forgejo, les trois fichiers cités sont propres par rapport au commit audité. Le binaire Go/Forgejo n’a pas été compilé ou lancé dans ce lot.

Le reconciler local traitait auparavant le tag complet comme un seul segment, donc feature/v1.0 devenait feature%2Fv1.0 et la lecture API valide était refusée comme ResourceMismatch. src/forgejo.rs::encode_url_path_segments encode maintenant chaque composant individuellement en gardant /; expected_release_html_url l’utilise pour calculer l’URL à partir de la configuration de confiance. Le nouveau cas d’intégration release_tag_with_slash_matches_forgejo_path_escape_segments lie le payload feature/v1.0, la lecture API courante et l’URL canonique.

## Preuves

Le test rouge a été reproduit en substituant temporairement l’appel antérieur encode_url_path_segment à l’unique site du reconciler. Il a échoué avec ResourceMismatch (code 101). Une restauration exacte vers encode_url_path_segments a ramené le SHA-256 du fichier à sa valeur pré-red-run : 33b87c15a2cf6ac2263095fa12b7e031d949d4aa84eaccdbfba3d2643f6dfa4f. Le journal est red-failing-tests.log; red.exit contient 101.

Après restauration :
- test ciblé release_tag_with_slash_matches_forgejo_path_escape_segments : 1/1;
- cinq suites webhook (webhook_reconcile_contract, webhook_reconcile_pipeline, webhook_contract, webhook_handler_contract, webhook_http_contract) : 154/154, 0 échec. Le test dédié est également inclus dans la suite contractuelle;
- rustfmt --edition 2024 --check sur les trois fichiers touchés et git diff --check sur ces fichiers : code 0;
- vérificateur de politique Cargo et ses tests : 12/12. L’état reste license=null, license_file=null, publish=[], no_license_selected, droits de redistribution not_assessed, SPDX NOASSERTION.

Le premier contrôle du manifeste append-only a identifié quatre empreintes anciennes (checkpoint.md, forgejo.rs, reconcile.rs et le test contractuel), sans chemin absent ni entrée dangereuse. Après ajout des dernières empreintes, le contrôle final passe; son reçu est manifest-verify.json. Les sorties du contrôle initial et final sont conservées séparément.

Les commandes exactes, sorties et codes de retour sont conservés dans ce dossier. toolchain.txt relève rustc 1.95.0, Cargo 1.95.0 et Python 3.12.3; Go n’est pas disponible dans le PATH. source-sha256.txt empreinte les trois fichiers Rust concernés.

## Limites et suite

Le test utilise le FixtureApi du contrat local; il vérifie une réponse de lecture API synthétique contre le code source Forgejo audité. Ce n’est pas un test HTTP sur une instance Forgejo live, ni une preuve de parcours agent/LLM, d’intégration AgentOS ou de gain comparatif. Aucun code Forgejo ou GitSpace n’a été copié; aucune dépendance nouvelle n’a été ajoutée; aucun choix de licence produit n’a été pris. Le worktree reste fortement modifié et l’écart complet de fichiers n’est pas attribuable à ce seul lot.

Suite locale : poursuivre l’audit des contrats webhook/release et traiter uniquement une lacune distincte encore non démontrée; garder les conclusions de runtime live, sécurité d’exécution, parcours LLM et benchmark séparées de ces tests de fixture.
