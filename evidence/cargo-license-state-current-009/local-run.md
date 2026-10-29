# CARGO-LICENSE-STATE-CURRENT-009 — relecture locale du statut non choisi

Date : 28 octobre 2026.

## État vérifié

Le paquet unique agent-forge-core garde license et license_file absents du manifeste Cargo, publish=false. Cargo expose respectivement license=null, license_file=null et publish=[] dans ses métadonnées. La revue custom marque decision=no_license_selected, redistribution_rights=not_assessed et spdx_package_license_declared=NOASSERTION.

Le vérificateur local renvoie status=metadata_policy_verified. Les douze tests scripts.test_cargo_license_state passent, notamment les refus de licence choisie, de fichier de licence, de publication activée, de marqueur SPDX déclarant une licence et de droits présentés comme évalués.

## Portée et limites

Ce lot confirme la politique technique Cargo demandée : ne pas publier le crate avant qu’un choix/licensing review soit fait. Il ne choisit aucune licence pour le produit, ne donne aucun avis juridique et ne prouve pas les droits de redistribution du code GitSpace, Forgejo ou des dépendances. Aucun code applicatif n’a changé. Toutes les commandes sont offline/local; aucun service externe, courriel, secret ou instance Forgejo n’a été utilisé.

Les sorties brutes, Cargo metadata, environnement et commandes sont dans ce dossier. Le manifeste local et les index append-only worktree/projet sont générés par append_manifests.py.
