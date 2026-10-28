# CONTINUATION-262 — réaligner les harnais de réconciliation sur l’attestation

Date : 2026-10-27. Worktree : `.runtime/webhook-005-workspace`; HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était fortement modifié; aucun lien entre ces changements et le commit HEAD n’est revendiqué.

## Constat et correction

Quatre chemins locaux démarraient `forgejo-webhook-local` sans graine Ed25519, puis fournissaient un FD credential à `forgejo-reconcile-local` : runners simple et emitted, suppression de branche native, et scénario PR de MISSION-012. Depuis CONT-261, une livraison in-scope aurait été refusée avant la lecture du FD. Ces chemins génèrent maintenant une graine aléatoire dans l’état privé, dérivent la clé publique avec `--print-attestation-public-key`, démarrent le receiver signé et fournissent la clé publique au reconciler. Le receiver du parcours emitted conserve la même clé lors de l’arrêt/redémarrage pour replay/idempotence. Les appels single de l’adaptateur partagé lisent aussi une politique depuis un fichier d’état owner-only, régulier et sans hardlinks; le batch fournit la clé explicitement. Le credential reste transporté par FD et n’est pas ajouté à argv.

## Vérification effectuée

- `python3 -m unittest tests/test_forgejo_reconcile_attestation_harness_args.py` : 5 tests réussis. `subprocess.run` est simulé; ils valident la dérivation/forme de clé, la transmission public-key aux chemins single et batch, le chargement depuis l’état privé, le maintien du token hors argv et le refus d’une clé non canonique avant spawn.
- `--help` des quatre runners : réussi, sans lancer de serveur.
- Les cinq fichiers Python touchés compilent en mémoire; `git diff --check` passe.
- Une première exécution du nouveau test échouait sur une assertion de test qui supposait un index fixe pour le FD; l’assertion a été corrigée pour chercher `--host-credential-fd`, puis les cinq tests passent.

Aucune instance Forgejo, aucun receiver live, aucun LLM ou service externe n’a été démarré. Le test simule seulement l’appel subprocess et ne prouve donc pas le parcours receiver → journal signé → Forgejo réel. Les commandes et empreintes des sources observées sont dans les fichiers voisins; les manifestes append-only préservent le préfixe historique.

## Limites et suite

La clé publique de test est stockée dans l’état privé créé par le runner, pas fournie par un superviseur de production. Ces changements restaurent les harnais locaux et ne protègent pas une invocation contrôlée par un agent. Prochaine étape : exécuter chaque runner seulement dans un bac à sable Forgejo local jetable dont la politique de nettoyage et l’absence de données de production sont explicitement confirmées; séparer cette validation de toute comparaison de valeur avec un LLM.
