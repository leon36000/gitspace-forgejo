# CONTINUATION-223 — reprise du checkpoint MISSION-012 dans le scope hôte

Date : 2026-10-27. Exécution locale isolée seulement. Aucun secret ou donnée privée n’a été envoyé à un service externe; aucun LLM payant n’a été appelé.

## But et changement

CONTINUATION-221 avait composé le cycle MISSION-012 courant dans Bubblewrap sous cgroup systemd créé par l’hôte, mais sans crash/reprise durable. Le composeur MISSION-012 possède déjà un mode de test `--test-host-restart` : il injecte SIGKILL après l’effet d’une écriture MCP, attend l’expiration du lease, redémarre le host, récupère un intent durable et demande à l’adaptateur de consommer le checkpoint `unverified`. Ce chemin avait déjà des preuves processus isolées; cette reprise établit son fonctionnement dans le montage complet sous scope hôte.

Le wrapper `run_mission_012_supervised_private_netns_e2e.py` expose maintenant `--test-host-restart`, opt-in et local seulement. Il conserve le scope host attesté, les namespaces Bubblewrap et l’adaptateur déterministe. Il passe au composeur le mode restart et n’ajoute pas `--same-host-gate-barrier`, explicitement incompatible avec la branche de redémarrage. Le même-child gate reste attesté par le parcours sans crash CONT-221.

## Échec conservé puis correction

Le premier run `live-mcp-223` s’est arrêté fail-closed pendant le stage agent : `sandboxed test adapter did not prove the credential and network refusals`. Aucun gate, PR ou merge n’a eu lieu et le scope s’est nettoyé (`clean`). L’examen du code a identifié une omission précise : la branche de reprise reconstruisait les commandes adapter « crash » et « recovery » sans leurs six arguments `--probe-private-file`, alors que le validateur attendait, pour une admission webhook, les canaris `admission_source`, `forgejo_read_token`, `host_admission`, `invalid_forgejo_read_token`, `mission_journal` et `webhook_inbox`. Ce refus signale l’absence de preuve du sandbox, pas une route ou un secret effectivement atteint.

Le code passe maintenant le même ensemble de sondes à la première tentative interrompue, à la reprise et au chemin normal. Le test `test_sandbox_restart_adapter_command_keeps_all_private_file_probes` vérifie la construction de la commande; trois tests du wrapper vérifient que l’option restart retire la barrière incompatible et valide les champs critiques du reçu.

## Parcours réussi 223b

Le second run passe les 17 contrôles du wrapper, avec `verdict=passed`, `gate_verdict=merged`, une sortie bornée, aucun timeout et `outer_scope_cleanup=clean`. Les reçus bruts figurent dans [MISSION-012 223b](../mission-012-live-agent-webhook-continuation-223b-resume-scope/supervised-private-netns-observation.json) et les fichiers du même dossier.

- Issue Forgejo #1, admission webhook versionnée, base `ee15939f30feed882d6efb82fb9e62c11d948745`, exigences SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`.
- La base produit un vrai échec de test (`-2` au lieu de `10`). Le host MCP, lié au bail initial, atteint le point d’écriture puis reçoit SIGKILL. Il reste un intent, zéro résultat et la modification candidate exacte (SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`).
- Le runner attend l’expiration observée du lease (10 250 ms); le nouveau lease a un ID différent, fence 2 après 1 et un délai de reprise borné à 120 000 ms. Le même `operation_id` est résolu une seule fois; le journal contient un résultat, et l’état fourni au nouvel adaptateur est explicitement `unverified`.
- L’adaptateur reprend via MCP, relit puis rejoue l’opération attendue, refait les tests avant de produire une candidate. Seul `src/lib.rs` change; les tests candidats et revalidation pré-merge passent.
- PR #2 reçoit vérification indépendante; un merge sans revue est refusé (HTTP 405). La décision autorisée mène au merge `9eddc616aa791cf92d537d7cf8a9e2995f8581a9`; Forgejo lit `main` à ce même SHA. L’acceptation post-merge sur ce commit exact passe 2/2.
- Le candidat reste dans le sandbox réseau/PID séparé; routes API, Web et receive-pack épinglées sont bloquées et les six fichiers privés restent cachés. Le gate s’exécute après que le stage/restart adapter est sorti, car cette option ne tient pas le même enfant vivant à la barrière.

Les manifestes des deux exécutions sont vérifiés : le dossier refusé contient un seul reçu d’observation; le succès 223b lie 17 fichiers. Leur empreinte `SHA256SUMS`, les logs de contrôle, les tests et les sources sont dans ce dossier.

## Vérifications locales

- `test_mission_012_live_composed_e2e.py` : 18/18.
- `test_mission_012_live_mcp_agent_e2e.py` : 14/14.
- `test_mission_012_supervised_private_netns_e2e.py` : 3/3.
- `py_compile` des quatre scripts et trois fichiers de test : code 0.
- `git diff --check` : code 0.
- Garde de licence Cargo : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.
- Manifestes locaux refus/succès et index append-only du worktree/projet vérifiés dans les reçus indiqués.

## Limites et interprétation

Le modèle est `local-test-adapter`; `llm_invoked=false`, coût déclaré 0 USD; aucun LLM, AgentOS, Forge Code ou RAG n’est intégré dans ce run. Un seul scénario déterministe, `n=1`; il ne mesure pas de gain contre Forgejo + agent + CI conventionnels. Ce test n’éprouve pas une panne d’alimentation/disque ou une restauration multi-hôte; les fichiers privés et le journal restent des ressources locales distinctes, sans atomicité commune. Il ne prouve pas que toutes les preuves métier possibles sont invalidées après tout type de modification.

Le reçu Rust du gate garde `external_writer_exclusion=trusted_supervisor_policy_not_independently_verified`; l’autorité de merge contre tous les writers n’est pas établie. Le mode restart n’observe pas le gate pendant que le MCP est vivant; CONT-221 prouve cette propriété dans le chemin normal seulement. Root, acteurs privilégiés, noyau compromis, évasion de namespace et autres systèmes d’exploitation restent hors modèle. Aucun plafond CPU/mémoire/tâches général du wrapper n’est démontré par ce run.

La mission reste incomplète : droits/licences amont, runtime AgentOS ou LLM réellement autorisé, refus exhaustifs, merge CAS universel et benchmark apparié restent ouverts.
