# CONTINUATION-364 — SIGKILL entre allocation durable et premier claim

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Arbre déjà fortement modifié; aucun fichier préexistant n’a été supprimé ou remis à zéro. Le seul ajout de code de ce lot est un test dans `tests/mission_execution_journal_contract.rs`.

## Lacune examinée

CONT-363 déplace le refus d’un contexte périmé avant la réservation durable. La réserve qui reste après validation doit pouvoir reprendre après interruption. Avant ce lot, les contrats prouvaient séparément qu’une allocation rejoue après réouverture et qu’un claim existant est idempotent après reprise, mais aucun ne tuait un processus après l’allocation et avant le premier claim.

## Test processus

`allocation_reservation_recovers_after_process_termination_before_first_claim` lance le même binaire de test avec un helper `#[ignore]`. L’enfant écrit une seule génération avec `allocation_id=interrupted-admission`, publie un marqueur après le retour de l’append, puis attend. Le parent lit le journal et confirme qu’il contient seulement `mission_generation_allocated_v1`; il envoie ensuite `SIGKILL` et vérifie ce signal de sortie. Une nouvelle instance du journal rejoue le même identifiant et la génération 1 (`replayed=true`), effectue le premier `mission_execution_claim_v1`, puis une autre ouverture retourne `AlreadyClaimed` sans ajouter d’événement. Tous les fichiers du scénario résident dans un répertoire temporaire privé.

## Vérifications

- Test ciblé : `cargo test --offline --locked --test mission_execution_journal_contract allocation_reservation_recovers_after_process_termination_before_first_claim -- --exact --nocapture` — code 0, 1 réussi.
- Suite finale : `cargo test --offline --locked --all-targets -- --test-threads=1` — code 0, 38 groupes, 740 réussis, 0 échec, 169 ignorés. Log complet : `all-targets.log`; synthèse du code de sortie et des compteurs : `all-targets-summary.txt`.
- `cargo fmt --all -- --check` et `git diff --check` — code 0. Le premier contrôle rustfmt a échoué sur quatre retours à la ligne du nouveau test; `rustfmt-before-fix.log` le conserve. Après `cargo fmt --all`, les deux vérifications finales passent.
- `python3 scripts/test_cargo_license_state.py` — 12/12; `python3 scripts/verify_cargo_license_state.py` — `metadata_policy_verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`.
- Outils enregistrés dans `tool-versions.txt`; sources et docs canoniques sont liés par SHA-256 et index append-only.

## Migration GitLab PC4 et limites

L’utilisateur avait indiqué que les accès se trouvent dans son courriel. CONT-357/359 signalent qu’un PAT trouvé dans une sortie antérieure du connecteur a été exposé; il n’est ni relu ni réutilisé. Après la nouvelle indication, les vérifications sans authentification montrent : ping Tailscale PC4 réussi, MagicDNS sans réponse, TCP 443 et TCP 22 en timeout. Aucun `ls-remote`, login, lecture de refs, changement de configuration réseau, commit de migration ou push n’a eu lieu. Git reste sur la branche locale avec `origin` Forgejo loopback. La voie distante attend une résolution/route service accessible depuis PC6 et un credential révoqué/remplacé fourni par un mécanisme local sûr; ensuite il faudra lire les refs et auditer l’export avant une branche `codex/`.

Ce scénario démontre la reprise après mort d’un processus Linux local à ce point précis du journal; il ne simule pas perte d’alimentation, corruption ou perte des données du système de fichiers, panne machine, verrouillage distribué, Forgejo live, GitLab, AgentOS/LLM, PR/fusion ou gain comparatif. Le helper ignoré ne passe pas seul dans la suite normale; le test parent l’exécute comme sous-processus.
