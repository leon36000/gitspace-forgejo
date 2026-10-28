# CONTINUATION-325 — cible GitLab PC4 inaccessible et correction du prochain lot

Date : 2026-10-27. Worktree produit : `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Réconciliation des preuves schema v3

La suggestion précédente de refaire une admission Forgejo schema v3 était obsolète. Le rapport existant [CONTINUATION-314](../continuation-314-forgejo-v3-source-pr-admission-005/local-run.md) établit déjà une PR source d’exigences et un avis sur Forgejo local v17, une fusion de cette PR vers `main`, l’émission native Forgejo `issues/opened`, sa réception, puis une admission réelle schema 3 par le CLI. Le vérificateur personnalisé de ce lot confirme les préfixes et les derniers hashes : 11 chemins dans le manifeste du lot, 150 dans celui du worktree et 151 dans le manifeste parent. Ce parcours doit être réutilisé, pas répété.

Limites conservées de CONT314 : `dismiss_stale_approvals=false` sur la fixture, reviewer synthétique, un parcours d’admission (`n=1`), aucun agent ni PR de code, aucune vérification indépendante de code candidat ou comparaison. Les tests complets et Python consignés dans son rapport sont historiques et ne sont pas relancés ici. Le comportement v3 live est donc démontré dans cette portée; la propriété d’identité humaine du reviewer ne l’est pas.

## Accès au dépôt demandé sur PC4

Le dépôt cible communiqué par l’utilisateur est `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`. L’état Tailscale local indique PC4 en ligne et le ping Tailscale réussit. En revanche, `getent hosts pc4.tail0c55ff.ts.net` échoue; `git ls-remote` sans résolution épinglée échoue sur le nom d’hôte. Après résolution locale de l’adresse du peer, une tentative HTTPS `git ls-remote` bornée à huit secondes expire sans sortie. Les connexions TCP 22, 80, 443 et 8443 expirent après deux secondes chacune.

Ainsi, aucune ref, branche par défaut, permission Git ou contenu distant n’a été lu. Aucun remote local n’a été changé, aucun commit/migration n’a été créé et aucun octet n’a été envoyé. La destination n’a pas été écrasée. Le remote `origin` courant reste le Forgejo local de test.

Le checkout produit contient environ 22 Gio de fichiers présents, dont 13 Gio sous `.runtime` et 8,2 Gio sous `target`; ces répertoires sont des états d’exécution/build et ne font pas partie des fichiers Git transférables. Le checkout produit comporte 19 fichiers suivis modifiés et 13 298 fichiers non suivis/non ignorés (environ 168 Mo); le checkout parent, séparé, comporte six suivis modifiés et 650 fichiers non suivis/non ignorés (environ 5,4 Mo). Ces deux worktrees partagent l’entrepôt Git mais ont des HEAD différents. Avant toute migration, il faut préserver les deux états, inspecter la base et les refs PC4, décider d’une branche dédiée puis n’y transférer que les sources et preuves utiles, sans `.runtime`/`target`, secrets ni credentials.

## Suite

Action utilisateur nécessaire : rendre le service GitLab réellement joignable depuis ce poste par Tailscale sur son port HTTP(S), ou indiquer le port/URL correct. Ne pas modifier ACL ou pare-feu automatiquement. Après rétablissement, relire les refs et les droits du dépôt, choisir une branche dédiée sans force-push, puis migrer les deux états locaux de manière traçable.

En attendant, poursuivre localement l’action restante du checkpoint CONT323 : audit statique des writers de refs et d’état fusionné dans Forgejo v17, en distinguant les chemins API/Web, AutoMerge, manuel/force, Actions et receive-pack. Les preuves de parcours complet, droits/licences, refus cumulés et comparaison appariée restent distinctes et ouvertes.
