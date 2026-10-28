# MISSION-RAW-REQ-STATUS-UTF8-001 — préserver le statut HTTP malgré un corps non UTF-8

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, sans commit dédié et fortement modifié. Sources examinées et empreintes : `source-sha256.txt`; outilchain : `tool-versions.txt`.

## Défaut reproduit et correction

`LocalLoopbackForgejoSource::get` décodait en UTF-8 la sortie complète de curl (corps HTTP puis suffixe de statut) avant de traiter le statut. Un corps binaire pour HTTP 404 ou 503 provoquait donc `InvalidResponse`; l’adaptateur perdait le vrai statut, notamment la disponibilité transitoire d’un 503. Le corps d’une réponse raw 200 doit rester un `Vec<u8>` jusqu’au parseur du document concerné.

Le test processus `raw_requirement_reader_uses_http_status_before_body_utf8` sert des octets `0xff` avec trois statuts au vrai lecteur/curl : 404, 503 et 200. Rouge, le 404 est classé `InvalidResponse` au lieu de `NotFound` (exit 101). Le lecteur découpe maintenant le dernier saut de ligne et valide le suffixe de statut ASCII à trois chiffres avant de traiter le corps. Il mappe 404 vers `NotFound`, 5xx vers `Unavailable`, et retourne intact le corps binaire d’une réponse 200. Les parseurs de JSON/contexte restent responsables de refuser un contenu 200 non valide.

`invalid_utf8_requirement_is_refused_before_generation_allocation` vérifie au contrat d’admission qu’un document d’exigences non UTF-8 devient `mission_requirements_invalid`, après une lecture et sans claim/allocation dans le journal. Le refus 1 MiB/curl 63 du lot précédent reste couvert et est inclus dans la suite complète actuelle.

## Résultats vérifiés

- Test rouge ciblé : échec attendu, exit 101; 404 avec corps `0xff` était `InvalidResponse`.
- Test vert ciblé : 1 réussi; 404 binaire est `NotFound`, 503 binaire est `Unavailable`/réessayable, 200 binaire retourne les mêmes octets.
- `cargo test --offline --locked --bin forgejo-mission-local -- --test-threads=1` : 33 réussis, 0 échec.
- `cargo test --offline --locked --test mission_admission_contract -- --test-threads=1` : 16 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_mission_admission_contract -- --test-threads=1` : 13 réussis, 0 échec.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 498 réussis, 0 échec, 55 ignorés sur 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check` : code 0.
- Garde Cargo : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`; aucune licence choisie.

Les logs, codes de sortie, commandes, versions, HEAD, branche et empreintes sont dans ce dossier. Son manifeste local couvre tous les fichiers hors manifeste lui-même et passe `sha256sum -c`; les hash courants sont ajoutés au manifeste global append-only.

## Portée et limites

Les réponses viennent d’un listener HTTP synthétique loopback et le client curl est le binaire réel; aucune instance Forgejo n’est lancée. La suite d’admission webhook prouve le contrat adjacent mais utilise son source fixture; ce lot ne compose pas le lecteur HTTP mission et l’allocation dans le même processus. Les 55 tests ignorés ne sont pas exécutés par `--all-targets`; certains E2E ont leurs runners Bubblewrap séparés. Aucun AgentOS/modèle configuré n’a été constaté dans les tests examinés; aucune intégration LLM, dépense, donnée privée, production ou service externe n’est utilisée.

Prochaine action : composer dans un test isolé la réponse raw 200 non UTF-8 jusqu’au refus d’admission sans journal de claim, puis sélectionner un refus webhook/réconciliation composé encore manquant. La comparaison contrôlée Forgejo+agent+CI et le parcours LLM restent non démontrés; projet incomplet.
