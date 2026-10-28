# MISSION-RAW-REQ-ADMISSION-COMPOSE-001 — refus HTTP raw 200 non UTF-8 avant allocation

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, sans commit dédié et fortement modifié. Sources/toolchain exactes : empreintes dans `source-sha256.txt`, versions dans `tool-versions.txt`.

## Parcours composé

`non_utf8_raw_requirement_through_http_source_refuses_admission_before_claim` appelle `admit_and_claim_mission_v2` avec le vrai `LocalLoopbackForgejoSource`, qui lance le curl produit contre un serveur HTTP synthétique loopback. Le fixture répond successivement : issue ouverte à l’URL canonique, branche `main` protégée au commit complet de fixture, puis HTTP 200 contenant un seul octet invalide UTF-8 (`0xff`) sur le chemin `raw/commit/{commit}/requirements/REQ-001.json`.

Le lecteur préserve le corps brut; l’admission le passe au parseur des exigences, qui retourne `mission_requirements_invalid`. Le test vérifie que les trois seuls GET attendus ont été servis, l’erreur est non réessayable et le journal d’exécution ne contient ni allocation ni claim. Le contexte de test est construit à partir d’une exigence valide, afin d’isoler la réponse invalide renvoyée par la source.

## Résultats vérifiés

- Test composé ciblé : 1 réussi, 0 échec.
- `cargo test --offline --locked --bin forgejo-mission-local -- --test-threads=1` : 34 réussis, 0 échec.
- `cargo test --offline --locked --test mission_admission_contract -- --test-threads=1` : 16 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_mission_admission_contract -- --test-threads=1` : 13 réussis, 0 échec.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 499 réussis, 0 échec, 55 ignorés sur 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check` et le garde Cargo passent.
- Garde de licence : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`; aucune licence n’est choisie.

Les logs, codes, commandes, versions, HEAD, branche et hashes sont conservés ici. Le manifeste local couvre les artefacts hors manifeste lui-même et passe `sha256sum -c`; les empreintes courantes sont ajoutées en fin du manifeste global append-only.

## Limites et suite

Le test traverse le vrai client HTTP et l’admission, mais le serveur Forgejo est simulé par un listener loopback. Il ne démontre pas le comportement de l’instance Forgejo réelle ni la création d’une PR. Les contrats webhook connexes utilisent leur propre source fixture; ce nouveau scénario ne rejoue pas le receiver ni la réconciliation d’un événement. Les 55 tests ignorés ont leurs runners isolés et n’ont pas tourné dans `--all-targets`.

Aucun fournisseur AgentOS ou LLM réel n’a été constaté; la fixture est explicitement locale et n’invoque pas de modèle. Aucun coût, service externe, donnée privée, changement de production, publication ou licence n’est engagé. Prochaine action : choisir un refus composé webhook/réconciliation encore absent dans la liste vérifiée du checkpoint, puis avancer séparément la récupération LFS scratch. LLM et benchmark contrôlé restent non démontrés; projet incomplet.
