# CONTINUATION-242 — refuser l’admission tant qu’une ambiguïté reste ouverte

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun reset, nettoyage, commit ou changement de production.

## Résultat

Le schéma Requirements v2 possède un tableau `ambiguities`, mais le code d’admission le validait seulement comme des chaînes non vides. Le run rouge a montré qu’un document contenant « Should bare integer values mean milliseconds or seconds? » retournait une admission, allouait la génération 1 et enregistrait `Claimed`.

`src/mission_admission.rs::prepare_admission` refuse maintenant les ambiguïtés non résolues après validation de la version et du SHA des exigences protégées, avant d’appeler l’allocateur. Le refus est `mission_requirement_ambiguity_unresolved`, `retryable=false`; il expose le chemin de résolution et le nombre d’éléments, sans recopier leur texte. `tests/mission_admission_contract.rs::unresolved_human_ambiguity_refuses_admission_before_generation_allocation` vérifie une lecture d’issue et d’exigence, l’actionnable, le compteur et un journal sans allocation ni claim. La révision d’exigences sans ambiguïté reste admise par le contrôle positif existant.

## Vérifications

- RED : le test ciblé, avant la garde, a échoué en recevant une admission complète (`execution_generation=1`, claim `Claimed`) au lieu de l’erreur attendue. L’exit code 101 correspond au comportement absent, pas à un défaut de compilation.
- GREEN ciblé : `cargo test --offline --locked --test mission_admission_contract unresolved_human_ambiguity_refuses_admission_before_generation_allocation -- --exact` — 1/1.
- Contrat complet : `cargo test --offline --locked --test mission_admission_contract` — 19/19.
- Suite : `cargo test --offline --locked --all-targets --quiet -- --test-threads=1` — 676 réussis, 0 échec, 134 ignorés sur 36 groupes; les tests ignorés ne sont pas couverts.
- `cargo fmt --all -- --check`, `git diff --check -- docs/architecture.md docs/checkpoint.md` et le contrôle d’espaces sur les cinq fichiers modifiés passent.
- `python3 scripts/verify_cargo_license_state.py` donne `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION`.
- Un premier appel direct au script de garde a retourné 126 (`Permission denied`); l’appel documenté avec `python3` passe. Les deux reçus sont conservés.

Les sorties brutes des suites et des gardes sont dans ce dossier; les commandes, versions et empreintes source sont également consignées ici. Le journal d’exécution Rust est temporaire et la fixture source est locale; aucune instance Forgejo n’a été lancée pour ce changement.

## Décision et limites

Les ambiguïtés restent dans le document versionné jusqu’à l’arbitrage humain. Le mainteneur publie ensuite une exigence révisée qui exprime la décision dans `request`/`acceptance` et ne contient plus d’élément ouvert. Le système n’infère pas le choix à la place de la personne.

Cette tranche ne journalise pas durablement le refus : elle ne peut pas comparer la révision aux tentatives précédentes et ne garantit donc pas que `revision` a augmenté entre le refus et une admission ultérieure. Le schéma ne stocke pas encore `decided_by` ou une justification structurée; l’historique Git/Forgejo demeure la seule trace. La vérification est au niveau contrat avec `FakeSource`, pas sur Forgejo live ou un LLM. Elle ne démontre pas le parcours complet, l’autorisation du décideur, une mesure de coût, une comparaison ou un gain général. La mission reste incomplète.
