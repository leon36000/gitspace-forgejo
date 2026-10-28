# CONTINUATION-097 — refus d’admission multi-writer sans CAS obligatoire

Date : 26 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre fortement modifié, donc ce HEAD n’identifie pas à lui seul les sources testées. Outils et SHA-256 des sources dans `tool-versions.txt` et `source-sha256.txt`.

## Modification

`src/forgejo.rs` définit deux profils d’admission du writer : `LocalSingleWriterSandbox` et `ForgejoV17MultiWriter`. La validation refuse le deuxième avec `server_expected_base_precondition_unavailable`, car l’audit épinglé de Forgejo v17 n’a pas établi de précondition atomique obligatoire sur tous les writers. Le CLI `--merge-local-test` prend `--writer-admission-profile`; s’il est omis, `forgejo_v17_multiwriter` est choisi et refusé avant la lecture du credential hôte. Le host doit sélectionner explicitement `local_single_writer_sandbox` uniquement quand il impose réellement l’exclusion de tous les autres writers. Le résultat est `inconclusive`, non autorisé, et sans tentative de merge.

Les capacités JSON exposent cette règle et son origine. Elles disent aussi que l’adaptateur ne vérifie pas lui-même l’exclusion des writers externes. La déclaration single-writer reste donc une exigence de configuration du host; elle ne constitue pas une attestation d’isolation. Ne pas l’utiliser hors du bac à sable réellement imposé.

## Vérifications

TDD : avant l’implémentation, `cargo test --offline --locked --test forgejo_merge_writer_admission` échouait à la compilation avec E0432 car les trois symboles d’admission attendus n’existaient pas. Un test CLI rouge séparé, `merge_cli_defaults_to_refusing_multiwriter_before_reading_merge_credential`, échouait en tentant de lire le FD inexistant au lieu de refuser le profil par défaut. Un all-targets intermédiaire a ensuite exposé un ancien test de parser qui n’indiquait pas le nouveau profil sandbox; `all-targets-red-01.log` conserve cet échec et le test passe désormais avec la sélection explicite. Après correction, les deux tests du contrat et les tests ciblés du gate/CLI passent.

Le test `merge_cli_defaults_to_refusing_multiwriter_before_reading_merge_credential` appelle le parseur/dispatch CLI réel avec le profil absent et un FD invalide; il obtient un refus JSON sans lire le FD. `run_cli_refusal.py` répète ce contrôle via le binaire CLI compilé, puis passe explicitement `local_single_writer_sandbox` et observe l’échec attendu de validation du FD invalide. Cette deuxième branche confirme le parsing de l’option, pas l’enforcement de la sandbox. Le test `v17_multiwriter_profile_is_refused_before_lock_inputs_or_forgejo_reads` appelle le chemin d’autorité de fusion avec le profil multi-writer; il confirme qu’aucun verrou n’est créé et qu’aucune entrée d’autorité n’est atteinte. L’ordre de contrôle de flux place le refus avant le code HTTP. Le CLI séparé n’a pas contacté de serveur et n’a reçu aucun vrai credential.

Commande complète : `cargo test --offline --locked --all-targets` — code de sortie 0, 35 groupes, 601 réussis, 0 échec, 100 ignorés. Sortie : `all-targets.log`. Le binaire est compilé par `cargo build --offline --locked --bin forgejo-reconcile-local`; le test CLI isolé est `python3 evidence/continuation-097-v17-multiwriter-admission-refusal-001/run_cli_refusal.py`.

Vérifications supplémentaires, codes 0 : `cargo fmt --all -- --check`; `git diff --check`; `cargo clippy --offline --locked --all-targets`; `python3 scripts/verify_cargo_license_state.py`; `python3 -m py_compile scripts/run_mission_012_live_agent_pr_e2e.py scripts/run_forgejo_live_pr_merge_e2e.py evidence/continuation-097-v17-multiwriter-admission-refusal-001/run_cli_refusal.py evidence/continuation-097-v17-multiwriter-admission-refusal-001/update_manifests.py`. Clippy garde des avertissements sur des fonctions/helpers dans plusieurs modules; ce run n’utilise pas `-D warnings`. L’état de licence Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.

## Portée et limites

- Le refus multi-writer est vérifié par un contrat Rust et un test unitaire du gate, `n=1` pour le chemin intégré; ce n’est pas un test de la route réelle Forgejo.
- Le profil single-writer est une option CLI explicite choisie par le host, pas une attestation. Les sources locales du superviseur MCP et du test adapter contrôlent l’egress du child agent, mais ne fournissent pas d’attestation excluant tout writer Forgejo API, Web, Git, admin ou processus local. La course base-POST de CONTINUATION-096 reste ouverte pour les writers externes.
- Aucun CAS Forgejo n’est simulé ou adopté. Aucun code Forgejo/GitSpace n’a été copié. Aucun serveur live, LLM, AgentOS, secret, coût ou service externe n’a été utilisé.
- Ce lot ne ferme ni le parcours LLM, ni la comparaison Forgejo+agent+CI, ni l’inventaire juridique de GitSpace. La mission globale reste incomplète.

Prochaine action : ajouter une attestation issue par le superviseur de l’ensemble Forgejo+gate isolé, ou refuser aussi l’option single-writer tant qu’elle n’existe pas. Les scripts existants attestent seulement l’isolement du child agent et ne suffisent pas. Garder le mode multi-writer refusé et le merge non qualifié; continuer ensuite les critères cumulatifs ouverts sans répéter les parcours déjà prouvés.
