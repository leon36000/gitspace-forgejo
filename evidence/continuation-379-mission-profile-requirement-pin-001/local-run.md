# CONTINUATION-379 — pin hôte des exigences MISSION-012

Date : 28 octobre 2026. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Écart reproduit

Le profil `mission-012-clamp` imposait déjà issue, exigence, révision, workspace et dépôt, et le host MCP vérifiait les critères compilés plus tard. À l’admission, le CLI acceptait néanmoins n’importe quel digest SHA-256 bien formé fourni avec le delivery; il lisait aussi les octets courants de Forgejo sans les comparer à ceux compilés dans le host MISSION-012.

Test-first au niveau processus isolé :

- Avec un digest différent, le CLI atteignait `webhook_inbox_missing` au lieu de répondre `mission_profile_requirement_mismatch`; la vérification du choix hôte était trop tardive.
- Une fixture Forgejo servant des critères différents mais valides avec `revision=1` a été admise avant le correctif : `verdict=claimed`, `execution_generation=1`, puis handoff contenant les critères modifiés. Le test a alors échoué sur le claim inattendu.
- La première forme de ce second test s’est arrêtée sur `mission_source_invalid_response`, car sa fixture d’arbre n’incluait pas le champ `sha` devenu obligatoire en CONTINUATION-377. La fixture a été ajustée pour annoncer le commit servi; le rerun a reproduit le défaut d’admission ci-dessus.

## Correction

`LocalAdmissionProfile` porte maintenant un champ optionnel `pinned_requirements`. Le profil MISSION-012 pointe sur les octets inclus avec `include_bytes!` du même JSON de fixture que `mission-012-mcp-host`; le profil `duration-parser` garde `None`.

`validate_profile_requirement_sha256` compare le digest caller-supplied lors du parsing des arguments avant toute lecture d’inbox ou de credential. Après lecture de la branche protégée et du document brut exact, le CLI compare aussi son SHA au même pin avant de charger le contexte, l’arbitrage, le journal de mission ou le handoff. Une divergence renvoie `inconclusive/mission_profile_requirement_mismatch` avec les deux empreintes, sans allouer.

La fixture HTTP MISSION-012 du test processus permet désormais de changer le document courant et renvoie le SHA de l’arbre pour respecter le contrat de pagination CONT-377. Le runner de processus inclut explicitement les deux nouveaux scénarios.

## Vérification finale

- `mission_profile_rejects_caller_digest_before_inbox_credential_or_allocation`: 1/1 sous Bubblewrap (`--unshare-net --unshare-pid`); erreur de pin avant l’inbox, le credential et l’allocation.
- `mission_profile_refuses_unpinned_current_requirements_before_allocation`: 1/1; la réponse API de critères modifiés est refusée après exactement les deux lectures attendues (branche puis requirements), sans lecture de `src/lib.rs`, journal/génération ou handoff.
- `actual_mission_cli_uses_private_host_credential_only_for_fixed_forgejo_reads`: 1/1; l’admission positive sur les critères compilés reste fonctionnelle et le token synthétique reste absent des sorties.
- `cargo test --offline --locked --bin forgejo-mission-local`: 48/48.
- `cargo fmt --all -- --check`, `git diff --check`, compilation syntaxique Python, les 12 tests de `scripts/test_cargo_license_state.py` et `scripts/verify_cargo_license_state.py`: passent.
- Cargo rapporte toujours `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, et SPDX `NOASSERTION`.

Les commandes sont dans `verification-commands.txt`; la sortie finale processus est `mission-profile-process-e2e-final2.log`, les 48 tests unitaires sont dans `forgejo-mission-local-unit-tests-final.log` et l’état de licence dans `cargo-license-state-final2.json`. Les empreintes, versions et hash du snapshot des sources utilisées par le runner se trouvent dans `environment.json`.

## Portée et limites

C’est un profil local de démonstration lié à des octets de fixture à la compilation. Le test HTTP utilise une fixture loopback isolée, pas Forgejo distant. Il n’établit pas l’authenticité ou l’atomicité de l’API, la protection humaine/administrative de la mise à jour des critères, la provenance attestée du binaire déployé, l’intégration réelle d’AgentOS/LLM, une PR, une fusion, ou un gain mesuré. Le profil générique `duration-parser` ne reçoit pas ce pin. Aucun code Forgejo/GitSpace n’est copié; aucun email/GitLab, commit, push ou dépense externe n’a été utilisé dans ce lot.

Le projet reste incomplet. Étape suivante : définir et tester une transition hôte auditée pour mettre à jour exigences et policy, puis relier cette sélection au binaire/configuration qui démarre le vérificateur; l’agent ne doit pas devenir l’autorité de rotation.
