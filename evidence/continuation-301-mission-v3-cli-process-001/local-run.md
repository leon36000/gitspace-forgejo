# CONTINUATION-301 — test du processus CLI d’admission schema v3

Date : 27 octobre 2026. Worktree fortement modifié, HEAD de base examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, reset, déploiement, publication, accès externe ou changement de production. Ce lot n’altère pas le choix de licence et ne copie pas de code Forgejo ou GitSpace.

## Question vérifiée

CONTINUATION-299 avait des tests du lecteur HTTP et de la politique v3, mais aucun processus CLI complet sur cette branche. Le test ajouté dans `tests/test_mission_requirement_arbitration_cli_process.py` compile et exécute le vrai binaire, avec une fixture locale pour tous les endpoints nécessaires à une admission `REQ-001` schema 3. Il vérifie le chargement de la politique host-only fixe, le binding au PR, le double readback PR/revues, l’allocation, ainsi que deux refus avant nouvelle allocation.

Pour ne pas ouvrir ou écrire le `.runtime` du dépôt produit, le runner copie uniquement `Cargo.toml`, `Cargo.lock` et `src/` dans un `TemporaryDirectory` sous `/tmp`. Cargo compile hors ligne à partir de cette copie. Le runner remplace, dans la copie seulement, la constante d’origine du profil `duration-parser` par l’origine loopback d’un port éphémère réservé par son serveur HTTP synthétique. Le `.runtime/mission-arbitration/duration-parser.json` et le journal `mission-execution.jsonl` sont donc exclusivement dans ce répertoire temporaire. Le serveur ne sort pas de `127.0.0.1` et ne transporte aucun credential.

## Résultats

- Succès : le processus sort avec code 0 et produit `admission_schema_version=3`. Il lie le document protégé synthétique au digest `13e235d9e33949921e804adde4d53bfd53b87989a3e2413399e54d9a97297b24`. L’avis du compte fixture `9001` est autorisé par une politique de scope exact; le CLI lit deux fois la PR #77 et deux fois le snapshot de revues avant l’allocation. Le journal temporaire existe après succès. `agent_runtime_invoked=false`.
- Politique absente : le second processus sort en échec avec `mission_arbitration_policy_missing`. Il ne lit que la branche et le document courant avant le refus : aucune route PR ou revue n’est demandée. Les octets du journal temporaire restent strictement identiques.
- Compte non autorisé : après remise en place de la politique, le fixture renvoie l’avis du compte `9002`. Le processus sort en échec avec `mission_requirement_arbitration_insufficient`; une seule lecture de l’instantané PR/revues est nécessaire pour établir zéro approbation autorisée. Les octets du journal ne changent pas.
- Résumé détaillé des verdicts et chemins HTTP : `cli-process-final4.log`. Les tentatives précédentes sont conservées, sans être la preuve principale; `cli-process-final4.log` est le run reproductible final utilisant un port éphémère.

## Commandes et vérifications

`python3 -m py_compile tests/test_mission_requirement_arbitration_cli_process.py` réussit. `python3 tests/test_mission_requirement_arbitration_cli_process.py -v` réussit : **1 test processus / 1 passé**, 6,72 s. Son sous-processus compile `forgejo-mission-local` avec `cargo build --locked --offline --bin forgejo-mission-local`; aucune requête Cargo réseau n’est permise.

`cargo test --locked --offline --bin forgejo-mission-local` : **43 réussis, 0 échoué, 0 ignoré**. `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` réussissent. Le garde confirme `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed` et `NOASSERTION`.

Versions et empreintes du code sont dans `tool-versions.txt` et `source-sha256.txt`; les sorties de commandes sont conservées dans ce dossier. Le runner ne copie aucun contenu du `.runtime` produit et ne consomme ni modèle ni clé.

## Portée et limites

Ceci teste le chemin du vrai binaire CLI en processus contre des réponses et comptes synthétiques; ce n’est pas un serveur Forgejo ni une revue humaine authentifiée. L’ID `9001` n’est qu’une valeur de fixture. La politique est un fichier synthétique à scope fixe; aucune policy de réviseurs réelle n’est trouvée ou modifiée. Le runtime agent n’est pas appelé et aucune tâche logicielle n’est exécutée. Le test ne prouve ni un gain comparatif, ni l’isolation d’un hôte de production, ni que Forgejo expose un type humain/bot. Pas d’AgentOS/LLM, d’instance Forgejo live, de coût ou de donnée privée impliqués.

La mission reste incomplète. Prochaine action : choisir dans la matrice un autre refus cumulatif encore insuffisamment démontré, puis reprendre les critères non établis d’exécution LLM/AgentOS et de comparaison conventionnelle uniquement si des interfaces et budgets autorisés sont disponibles.
