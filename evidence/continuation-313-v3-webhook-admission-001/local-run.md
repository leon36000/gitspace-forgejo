# CONTINUATION-313 — admission schema v3 après livraison webhook signée

Date : 27 octobre 2026. Worktree produit très modifié, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit, reset, nettoyage ou changement de production.

## Lacune vérifiée

La preuve CONT-301 lançait le processus `forgejo-mission-local` avec des exigences schema v3, une politique d’approbation privée et une API HTTP synthétique, mais sans livraison webhook. Les tests de processus receiver→CLI signés existants utilisaient des exigences schema v2. La nouvelle couverture combine ces interfaces dans un test processus, sans prétendre qu’une fixture est une instance Forgejo.

## Test et résultat

Le test `tests/forgejo_mission_delivery_process_e2e.rs::actual_mission_cli_admits_signed_webhook_with_v3_requirement_arbitration` démarre le receiver Rust, lui envoie une livraison `issues/opened` HMAC valide, puis lance le vrai binaire `forgejo-mission-local` avec cette livraison et le SHA des exigences schema 3. La politique d’approbation de test est écrite sous `.runtime/mission-arbitration/duration-parser.json` dans le `.runtime` scratch fourni par le runner, avec modes répertoire `0700` et fichier `0600`. Le serveur API loopback renvoie l’issue courante, la tête protégée, les exigences, une PR source #77 fusionnée, puis une liste paginée avec `X-Total-Count: 1` et une approbation liée au head provenant de l’ID synthétique `9001`.

Test-first : le premier test processus échoue au retour CLI `mission_source_invalid_response`, car la fixture initiale n’avait pas les routes PR/revues consommées par l’admission v3. Après ajout de ces seules réponses synthétiques, le test ciblé passe `1/1`. Une compilation intermédiaire du harnais a signalé une utilisation après déplacement de `String`; elle a été corrigée en conservant une copie du chemin avant de le relire. Le contrôle `cargo fmt --all -- --check` a ensuite identifié seulement le format des lignes ajoutées; après formatage du fichier concerné, le contrôle final passe.

La sortie finale indique `admission_mode=webhook_delivery`, `admission_schema_version=3`, livraison `mission-v3-delivery-1`, PR source 77, reviewer `[9001]`, le digest exact des octets et `agent_runtime_invoked=false`. Le journal mission et le handoff host existent dans le scratch. L’exigence et la politique ne sont pas transmises par l’agent; le CLI les lit via ses sources configurées.

## Commandes et provenance

Commande de preuve finale : `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_admits_signed_webhook_with_v3_requirement_arbitration`. Le runner compile hors ligne la cible et les binaires requis, puis l’exécute sous Bubblewrap avec namespaces réseau/PID distincts, `.runtime` tmpfs privé, home masqué et limite de ressources. Le log complet est dans `green-test.log`; la sortie inclut `source_tree_sha256=8d45fadbdcab3903358194063a2571d487bffda6511a4ed14324bbfd312bc33a` et le hash du runner `5825c7a213fab314e0e7b81bd5546a55b774a4ddfe15bd35ec56bf6c12fdb665`.

Environnement relevé : Rust/Cargo 1.95.0, Bubblewrap 0.9.0, Linux 7.0.0-1019-nvidia-aarch64 avec glibc 2.39. `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le garde confirme encore `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed` et marqueur `NOASSERTION`; aucune licence n’est choisie.

## Limites

Le receiver et le CLI sont des processus Rust réels dans le sandbox, mais la source Forgejo est un serveur HTTP fixture sur loopback. La PR, le reviewer, la tête et les exigences sont synthétiques; l’ID d’un avis ne prouve pas une identité humaine ou l’absence de bot. Un seul run (`n=1`) est effectué. Aucun serveur Forgejo, AgentOS, LLM, agent exécutant, changement de code, PR de code, vérification indépendante, décision de fusion ou merge n’a été utilisé dans ce lot. Aucun gain de temps, tokens ou coût n’est mesuré. Cette preuve ferme seulement le raccord de test livraison signée→admission schema v3; elle n’achève pas la mission produit.
