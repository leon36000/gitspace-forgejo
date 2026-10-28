# FORGEJO-MERGE-AUTHORITY-REQUEST-ALLOWLIST-001 — résultat local (25 octobre 2026)

## Cible et portée

Le test porte sur le client Rust LocalLoopbackForgejoApi dans le worktree local. HEAD est f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, branche local/WEBHOOK-005-canonical-links; le worktree est fortement modifié et src/bin/forgejo-reconcile-local.rs est modifié. Le SHA-256 du fichier entier est consigné dans source-hashes.txt : il identifie l’entrée observée, pas une version contenue dans HEAD.

La politique MergeAuthority est vérifiée dans request_api_once avant la lecture du credential, la construction de l’URL et l’appel curl. Elle autorise uniquement les formes de lecture utilisées par le gate :

- GET /user et GET /users/{login} avec un login syntaxiquement borné;
- GET de la PR sélectionnée, ses pages d’avis, de la branche main et de l’issue;
- GET des commits et arbres Git complets, avec les paramètres de pagination exacts;
- un POST /repos/{owner}/{repository}/pulls/{number}/merge sans query string.

Le corps de ce POST est décodé dans une structure Serde avec deny_unknown_fields. L’action doit être merge et head_commit_id doit être un identifiant d’objet Git complet. Les champs absents, les champs supplémentaires ou dupliqués, une autre action, une méthode/route hors liste, une pagination non canonique, une query string de merge et un autre dépôt sont refusés.

Le filtre de route ne lie pas à lui seul le numéro de PR ou le SHA du corps aux preuves. Le gate calcule les valeurs depuis l’état vérifié avant l’appel; cette assertion demeure dans le code appelant. Surtout, le filtre est dans un processus client : il ne restreint pas l’egress du poste ni un autre processus et n’empêche pas un agent de joindre l’API, le Web ou Git directement si son environnement le permet.

## Résultats

Le test test-first appliqué à l’ancien prédicat body.is_some() échoue sur le corps Do=rebase, comme attendu. Après raccordement du parseur strict, cinq tests ciblés passent : la matrice de routes/corps, le refus avant réseau/lecture du credential, la porte de profil par défaut, la portée du profil MISSION-012 et la séparation credential mergeur/lecteur de politique. Les journaux finaux montrent chacun 1 test réussi; les tests de politique sont deux cas distincts.

Dans le test de side-effects, une tentative PATCH de branche et un POST merge dont Do vaut rebase retournent Forbidden sans lire le credential. La seule requête reçue par le serveur loopback est le GET de PR autorisé; le compteur du fournisseur de credential vaut un. Les parcours de profil utilisent aussi des fixtures HTTP loopback; ils ne démarrent pas Forgejo.

Cargo clippy termine avec code 0. Il affiche un avertissement too_many_arguments dans le helper de fixture start_profile_gate_fixture_with_role_credentials et d’autres avertissements dans des modules distincts; aucun n’est dans le parseur ou les fonctions de politique du lot. cargo fmt --all -- --check, git diff --check et scripts/verify_cargo_license_state.py terminent avec code 0. L’état licence demeure null/null, publish vide, décision no_license_selected, droits not_assessed.

Le premier journal body-policy-red.log est un essai de test invalide avec un délimiteur de chaîne brute mal formé; il n’est pas utilisé comme preuve comportementale. Le journal body-policy-red-behavior.log contient la reproduction valide où le filtre antérieur accepte Do=rebase; le journal body-policy-green-current.log contient le test après correction.

## Limites et suite

Aucun test ne démontre une barrière réseau, une séparation d’UID, une absence d’accès direct pour un agent, l’isolation d’un runner ou la non-exposition d’un credential à un processus hostile. Aucun Forgejo vivant ni LLM n’est sollicité; aucun token réel ou secret n’est utilisé. Les fixtures de profil ne remplacent pas la preuve d’un environnement d’agent réel. Ce lot n’établit ni une fusion contrôlée complète ni un gain comparatif.

La prochaine preuve doit utiliser un bac à sable local avec une frontière d’exécution observable : le gate conserve seulement les routes autorisées et le credential; un processus agent séparé tente API, Web et receive-pack et reçoit un refus de connexion ou d’autorisation sans credential accessible. Si aucune isolation existante n’offre cela, concevoir un broker à opérations structurées et une identité système réellement distincte, ou évaluer une permission serveur étroite. Ne pas traiter une allowlist client comme une frontière de confiance.

## Artefacts reproductibles

Les commandes exactes, résultats, versions d’outils et empreintes figurent dans commands.txt, toolchain.txt et source-hashes.txt. Les sorties de tests, le rouge comportemental, le premier essai syntaxiquement invalide et Clippy sont préservés dans les fichiers .log de ce dossier; SHA256SUMS couvre les artefacts locaux.
