# CONTINUATION-185 — le test-adapter reconnaît l’issue non fiable

Date : 27 octobre 2026. Worktree observé à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, sale et fortement modifié; HEAD inchangé. Aucun commit créé.

## Lacune et changement

CONT-184 transmettait `untrusted_work_item` au processus MCP, mais `scripts/mission_012_test_agent_adapter.py` ne lisait que `requirements_json` et `resume_checkpoint`. `observe_untrusted_work_item` reconnait maintenant le schéma v1, l’origine/source, la marque `untrusted_user_content`, l’identité et la route d’issue, le champ `updated_at` non vide et borné, les formes `explicit_null` ou `text`, les limites de titre/corps et la forme hexadécimale du digest. Le contexte facultatif absent est accepté; une version booléenne `true`, un trust label erroné ou une enveloppe non conforme provoque un refus avant `cargo_test` et écriture.

Le texte du titre et du corps ne pilote jamais le critère, les commandes ou le contenu écrit; l’adaptateur n’émet qu’un message fixe disant qu’il a observé le contexte séparément. Le digest est contrôlé en forme, pas recalculé ici. Le host Rust a déjà validé le DTO avant de le servir; cette validation n’atteste ni la justesse du code ni la sûreté d’un futur agent.

## Rouge/vert et composition de processus

Le nouveau test Python `test_untrusted_issue_instructions_do_not_change_protected_work` a d’abord échoué sur le résultat : l’adaptateur ne signalait pas l’observation du champ. Cet échec exposait l’interface ignorée; il n’indiquait pas que la fixture avait accepté une nouvelle exigence. Le log rouge est conservé. L’empreinte du snapshot source au moment rouge n’a pas été prise; ne pas utiliser les empreintes finales comme preuve de ce snapshot historique.

Après correction, `python3 tests/test_mission_012_test_agent_adapter.py` passe **6/6**. La fixture MCP donne un titre demandant d’ignorer les exigences, un corps demandant de ne pas appeler `cargo_test` et de publier un faux succès, et des exigences protégées demandant la limite supérieure. L’adaptateur exécute tout de même l’observation rouge et la vérification verte, écrit exactement `SOURCE_AFTER`, et ne place pas les marqueurs hostiles dans sa sortie. Deux contrôles supplémentaires refusent un trust label différent et `schema_version: true` avant toute exécution/écriture; le test existant confirme aussi que le champ facultatif absent n’ajoute aucun message.

La commande `cargo test --offline --locked --features test-crash-injection --test mission_012_mcp_lifecycle sigkill_before_and_after_workspace_effect_are_recovered_under_a_new_fence -- --exact --nocapture` passe **1/1** (`31 filtered`). La fonction exerce deux interruptions SIGKILL, une avant l’effet filesystem et l’autre après. Après chaque redémarrage, le véritable processus `mission-012-mcp-host` fournit le contexte d’issue hostile créé par `claim_execution_with_untrusted_issue_content`; le vrai processus Python `mission_012_test_agent_adapter.py` reconnait ce champ, récupère le checkpoint `unverified`, reprend l’écriture autorisée et obtient le test d’acceptation vert. Le dernier run a duré 24,89 s.

## Vérifications et provenance

- `python3 tests/test_mission_012_test_agent_adapter.py` — 6/6.
- La commande Rust feature-gated ci-dessus — 1/1, 31 filtrés, deux points de crash dans le test.
- `python3 -m py_compile scripts/mission_012_test_agent_adapter.py tests/test_mission_012_test_agent_adapter.py` — réussite.
- Contrôle de blancs finaux des trois sources modifiées et des quatre documents — réussite.
- `cargo fmt --all -- --check` et `git diff --check` — réussite.
- `python3 scripts/verify_cargo_license_state.py` — `status=verified`; `cargo metadata --offline --locked --no-deps --format-version 1` relit package `agent-forge-core` v0.1.0 avec `license=null`, `license_file=null`, `publish=[]`.

Environnement et empreintes finales : `toolchain.txt`, `source-hashes.txt`; logs rouge/vert et état Cargo : fichiers voisins. Les sources concernées sont l’adaptateur, ses tests, le test de lifecycle MCP feature-gated, et les documents `architecture.md`, `checkpoint.md`, `implementation-plan.md`, `refusal-evidence-matrix.md`. Les reçus `SHA256SUMS` locaux et globaux lient les artefacts aux index append-only.

## Limites

Ceci prouve une consommation d’interface avec un adaptateur déterministe et un host local. Cela ne prouve pas qu’un LLM respecte les marqueurs de confiance, qu’une injection ne l’influencera pas, ni qu’AgentOS existe ou est intégré. Le processus test-adapter partage l’identité de l’hôte; aucune frontière de privilège/réseau n’est évaluée dans ce test. L’issue, les exigences et le dépôt sont synthétiques; aucun serveur Forgejo, webhook live, PR, revue ou fusion n’est impliqué. L’ancien parcours MISSION-012 complet n’est pas rejoué; le gain comparatif, les interventions humaines et les coûts en tokens/facturation restent non mesurés. Aucun service externe, secret, dépense, publication, changement de production, choix de licence ou commit.

Suite : composer si possible le contexte issue courant avec le parcours local complet déjà établi, sans LLM payant, ou poursuivre une cellule de refus non couverte; garder le benchmark apparié explicitement ouvert.
