# MISSION-012-HOST-RESTART-RECOVERY-001

Date : 25 octobre 2026. Worktree produit sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le dépôt reste fortement modifié; les sources runtime/host/tests ne sont pas suivies par Git. Aucun commit, publication ou déploiement n’a été effectué. Les empreintes des sources finales sont dans `source-sha256.txt`.

## Changement réalisé

Le journal expose maintenant une prélecture en lecture seule qui trouve au plus un intent `workspace.write_text` non résolu pour l’allocation et la requête exactes. Le host valide l’arbre de base admis, les exigences, l’identité de mission, le hash de la requête et le pré-état source. Il tolère un candidat modifié seulement si son manifeste complet ne diffère de la base que pour `src/lib.rs`, au hash avant ou après enregistré dans l’intent et avec le mode Git inchangé.

Après prise d’un lease neuf, le host revalide le statut de l’opération sous le nouveau fence. Si l’effet fichier avait eu lieu, il restaure `src/lib.rs` depuis la base par remplacement atomique; si l’effet n’avait pas eu lieu, il garde la base. Le contexte MCP comprend alors un checkpoint `unverified` lié à l’intent. L’agent doit rejouer l’écriture exacte; le host refait le test de baseline avant d’accepter la correction. Les autres fichiers, plusieurs intents, une base ou un hash tiers, et un intent déjà résolu ne sont pas récupérés par cette voie.

La feature Cargo `test-crash-injection` est désactivée par défaut. Elle tue le host après la persistance de l’intent, immédiatement avant ou après l’effet fichier, et est interdite en build release par `compile_error!`.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1` : 460 réussis, 0 échec, 30 ignorés, 34 groupes. Le contrat journal compte 64 réussis/2 ignorés; la suite lifecycle compte 25 réussis/1 ignoré.
- `cargo test --offline --locked --test mission_012_mcp_lifecycle mcp_host_restarts_from_an_exact_unresolved_workspace_write_intent -- --nocapture` : le vrai binaire local redémarre depuis un intent/effet pré-constitué, réobserve le rouge, rejoue l’input sous un nouveau fencing token et vérifie l’acceptation verte.
- `cargo test --offline --locked --features test-crash-injection --test mission_012_mcp_lifecycle sigkill_before_and_after_workspace_effect_are_recovered_under_a_new_fence -- --nocapture` : 1 test passe; deux sous-cas provoquent SIGKILL avant et après l’effet, attendent l’expiration du lease, puis reprennent le host. Résultat vert et unique sont vérifiés au nouveau fence.
- `cargo check --offline --locked --release --features test-crash-injection --bin mission-012-mcp-host` échoue sur le garde de compilation prévu : `test-crash-injection is restricted to debug builds`.
- Build release du binaire MCP, `cargo fmt --all -- --check`, et Clippy all-targets avec les six allowances préexistantes documentées (`too_many_arguments`, `collapsible_if`, `bool_assert_comparison`, `manual_div_ceil`, `enum_variant_names`, `type_complexity`) réussissent. Clippy passe aussi pour le test feature-gated `test-crash-injection` avec les mêmes allowances. `git diff --check` réussit pour les diffs suivis; rustfmt et la compilation couvrent aussi les sources Rust non suivies.
- `cargo metadata --offline --no-deps --format-version 1` rapporte `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; la feature de test est déclarée sans dépendance nouvelle. La décision de licence reste non sélectionnée.

Les commandes et sorties sont conservées dans ce répertoire; `SHA256SUMS` permet leur relecture locale. Les logs de tests précédemment acquis ne sont pas des parcours Forgejo ou des mesures comparatives.

Note de provenance après le lot 002 : le fichier `source-sha256.txt` de ce rapport est conservé avec les empreintes observées lors de l’exécution 001. Le lot 002 a depuis modifié `tests/mission_012_mcp_lifecycle.rs`, `docs/architecture.md` et `docs/checkpoint.md`; les empreintes 001 de ces trois chemins ne correspondent donc plus au contenu courant. La source actuelle de reprise d’adaptateur est hachée séparément dans le dossier 002. Le run 001 reste historique et n’est pas revendiqué comme reproductible sur l’arbre courant.

## Limites et suite

Le parcours SIGKILL est local et synthétique : il exerce de vrais processus MCP, mais ni coupure d’alimentation, ni processus malveillant du même UID, ni agent LLM. Il ne crée pas de PR et ne fusionne pas dans Forgejo; les preuves composées antérieures ne sont pas rejouées ici. Aucun AgentOS, modèle local, intégration Forge Code, RAG ou mémoire n’a été découvert dans les interfaces examinées. Aucun coût ou accès nouveau n’a été engagé.

Suite : composer uniquement cette branche de reprise avec le parcours local-adapter vers PR, vérification indépendante, décision motivée et fusion contrôlée; exercer les refus de base avancée, fichier supplémentaire, hash tiers et état déjà résolu. Le travail global reste incomplet tant que la démonstration LLM autorisée et la comparaison contrôlée avec Forgejo/agent/CI conventionnels manquent.
