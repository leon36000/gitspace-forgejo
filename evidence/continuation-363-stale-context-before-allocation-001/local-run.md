# CONTINUATION-363 — valider le contexte avant réservation

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste fortement modifié; ce lot est local, non commité et non publié.

## Défaut et modification

`prepare_admission` réservait une génération avant de construire et valider l’`AgentExecutionRequestV1`. Le contexte peut être périmé lorsque le snapshot source de contexte a été lu avant une avance de branche : le processus rend alors `context_stale`, mais avait déjà écrit `mission_generation_allocated_v1` alors qu’aucun claim ou handoff n’était possible.

`AgentExecutionRequestV1::validate_before_allocation` appelle le validateur partagé en sautant uniquement l’exigence que `mission_id` et `idempotency_key` soient non vides. Ces champs sont dérivés de l’allocation et restent vides dans l’objet local temporaire; ils ne sont jamais transmis. `prepare_admission` construit et valide toute la requête/contexte (requirements JSON, hash, arbitrage, base, budget, sources/citations, capacités et contexte d’issue) avant l’appel de réservation. Ensuite, il alloue, dérive l’identité réelle, remplit les deux champs et exécute le validateur normal avant le claim.

Le scénario processus existant a été renforcé : après `context_stale`, le journal doit être vide et aucun handoff présent; le retry du même delivery sur la nouvelle base lit un contexte frais, crée la génération 1 avec `allocation_replayed=false`, puis claim. Le test contractuel Rust vérifie aussi l’absence d’événement au refus puis l’allocation fraîche. Un E2E de deux admissions concurrentes a été rejoué après le déplacement de la validation : les deux processus utilisent la même entrée, avec un seul checkpoint de génération et un seul claim.

## Vérifications

- Test processus rouge : `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_rejects_stale_context_before_allocation_then_claims_after_base_advance` — avant correction le test observe l’événement de réservation, contrairement au contrat attendu (`stale-context-red.log`).
- Test processus vert : même commande après modification — 1/1, refus avant journal/handoff puis reprise fraîche (Bubblewrap `--unshare-net --unshare-pid`, fixture HTTP loopback) (`stale-context-green.log`).
- Test contractuel ciblé `failed_context_validation_leaves_no_reservation_and_can_retry_fresh` — 1/1 (`unit-contract-green.log`).
- Test d’admission concurrente `actual_mission_cli_concurrent_duplicate_deliveries_claim_one_generation_once` — 1/1; une génération et un claim (`concurrent-admission.log`).
- Suite finale : `cargo test --offline --locked --all-targets -- --test-threads=1` — code 0, 38 groupes, 739 réussis, 0 échec, 168 ignorés (`all-targets.log`).
- `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py` — code 0.
- Garde Cargo 12/12 et readback `metadata_policy_verified`: `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.

Un all-target run intermédiaire a échoué parce qu’un test unitaire encodait l’ancien comportement (réservation avant validation); le contrat a été mis à jour, puis la suite complète a repassé. Le log est conservé. Les versions, hashes de sources et index append-only sont dans le dossier.

## Portée et limites

La vérification évite les mutations durables pour les requêtes/contextes invalides avant allocation. Une fois toutes ces validations réussies, la réservation demeure un checkpoint récupérable pour les interruptions entre allocation et claim. Un défaut de stockage, un mismatch d’allocation ou un arrêt à ce point peut encore laisser une réservation; cette tranche ne répare pas les journaux corrompus.

Les tests processus utilisent le vrai receiver/CLI Rust, une fixture HTTP Forgejo locale, des données et secrets synthétiques, et Bubblewrap privé réseau/PID; `n=1` par scénario. Aucun Forgejo/GitLab live, email/secret réel, AgentOS, LLM, PR, fusion ou comparaison de valeur n’est exercé. Aucun coût ni appel réseau externe; aucune dépendance ou licence sélectionnée.

## Prochaine action

Comparer le chemin exact « allocation durable écrite, puis interruption avant claim » aux scénarios de crash/reprise déjà présents. Les réutiliser si leur preuve processus est à jour; sinon isoler cette frontière avec un arrêt contrôlé et vérifier que le retry garde la même génération et le même `allocation_id`, sans prétendre que la réservation équivaut à une mission exécutée.
