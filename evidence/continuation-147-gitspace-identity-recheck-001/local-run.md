# CONTINUATION-147 — identité publique du dépôt GitSpace

Date : 26 octobre 2026. Recontrôle en lecture seule du dépôt public exact fourni par l’utilisateur, depuis le worktree local; aucune copie de code, modification upstream, dépense, authentification ou donnée privée.

## Résultat

La commande sans credential `GIT_TERMINAL_PROMPT=0 GIT_CONFIG_NOSYSTEM=1 git -c credential.helper= ls-remote --symref https://github.com/leon36000/GitSpace.git HEAD refs/heads/main` retourne `HEAD` et `refs/heads/main` à `dcde1142cf76996a6ba0074cb7eab9d3c870c561`, identique au SHA déjà enregistré dans CONTINUATION-086. Le clone temporaire d’audit et les archives ZIP restent indisponibles dans les emplacements documentés; aucune empreinte d’archive n’est établie.

La page GitHub du dépôt demandé, consultée le 26 octobre, l’identifie comme `leon36000/GitSpace`; son README décrit un bootstrap documentaire en PR brouillon, affirme qu’aucun code, dépendance, licence ou CI produit n’est inclus et affiche `Licence UNKNOWN`. La liste de racine rendue ne montre pas de fichier `LICENSE`. Ce sont des déclarations/observations de page; elles ne remplacent pas l’audit antérieur des fichiers exécutables `crates/` au commit `dcde114` ni ne prouvent les droits sur les fichiers de l’archive absente.

Le nom a un homonyme distinct, `wieslawsoltes/GitSpace`, présenté comme un client Git C#/Uno sous MIT. Son identité, son code et sa licence sont étrangers à la source demandée et ne peuvent pas être transposés au dépôt `leon36000`.

## Références et portée

- Ref publiée : sortie exacte conservée dans `git-ls-remote.txt`; Git `git-version.txt`.
- Source primaire demandée : [leon36000/GitSpace](https://github.com/leon36000/GitSpace), dont le README affiché dit `Licence UNKNOWN`, situe le travail en bootstrap PR brouillon et distingue le corpus canonique du code produit.
- Homonyme distinct : [wieslawsoltes/GitSpace](https://github.com/wieslawsoltes/GitSpace), qui se présente comme un client Git C#/Uno et affiche MIT license.
- L’inventaire et le checkpoint ont reçu cette désambiguïsation. Aucun test logiciel n’était pertinent pour une vérification de source publique; aucun code GitSpace n’est importé. Les droits de réutilisation/redistribution demeurent indéterminés.

Les hashes source et index append-only sont dans `source-sha256.txt`, `SHA256SUMS` et les reçus de vérification de ce dossier. Le rapport ne constitue pas un avis juridique.
