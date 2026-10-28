# WEBHOOK-RECONCILE-UNKNOWN-DELIVERY-001 — refus structuré des IDs absents

## Défaut et correction

Les deux tests de régression ont d’abord échoué sur la même observation : pour un ID absent de l’inbox vérifiée, le CLI écrivait une erreur sur stderr au lieu de produire le résultat JSON prévu. Les journaux rouges `single-red.log` et `batch-red.log` conservent ces échecs avant la correction.

`run_reconciliation` résout maintenant tous les IDs demandés depuis l’inbox avant de lire un FD de credential ou d’effectuer une lecture amont. Si l’un des IDs manque, la demande est refusée comme un tout avec verdict `inconclusive`, `delivery_not_found`, une action de reprise et `upstream_read_attempts: 0`. En single, le résultat conserve `delivery_id`; en batch, il conserve `requested_delivery_count` et indique le premier ID absent. Le CLI sort avec code 1 et écrit le JSON sur stdout.

Le test single passe par le processus compilé et une inbox vide. Le test batch demande deux IDs absents et passe `i32::MAX` comme descripteur de credential invalide : il obtient le JSON structuré, zéro lecture amont et aucun message stderr. Ces assertions prouvent que la sélection a échoué avant toute tentative de lecture du descripteur; aucun secret ni credential réel n’est utilisé.

## Vérifications

- `cargo test --offline --locked --test reconcile_config_contract` — 4/4 réussis après formatage.
- `cargo test --offline --locked --bin forgejo-reconcile-local` — 105/105 réussis.
- `cargo fmt --all -- --check` — réussi après avoir formaté les deux nouvelles assertions.
- `git diff --check` — réussi.
- `python3 scripts/verify_cargo_license_state.py` — `status=verified`, `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`.

Versions, HEAD, hashes des sources, journaux rouges/verts, formatage, diff-check et readback Cargo sont conservés dans ce dossier. Le manifeste local `SHA256SUMS` couvre les artefacts du dossier.

## Portée et limites

Il s’agit de tests locaux du processus CLI avec une inbox temporaire vide, pas de Forgejo réel, webhook reçu, LLM, AgentOS, pull request, fusion, comparaison de productivité ou coût. Aucun accès réseau n’est exercé par ces cas. Le test batch couvre deux IDs absents; il ne démontre pas une demande mêlant un ID existant et un ID absent.
