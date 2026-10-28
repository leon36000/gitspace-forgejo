# WEBHOOK-HTTP-DISCOVERY-AND-HOST-VALIDATION-001 — preuve locale

Date : 25 octobre 2026. Ce lot apporte une découverte statique versionnée du récepteur et corrige deux entrées HTTP/1.1 ambiguës qui atteignaient auparavant l’ingestion.

## Comportements vérifiés

- Le processus réel `forgejo-webhook-local`, lié à un port loopback éphémère, répond `200` à `GET /forgejo/capabilities`. Le JSON `schema_version: 1` décrit l’endpoint d’ingestion, les alias d’en-têtes Forgejo/Gitea, HMAC-SHA256, les six événements réellement pris en charge, les limites d’en-tête/corps et les résultats idempotents `202 duplicate` et `409 delivery_conflict`. La lecture ne modifie pas l’inbox; `POST` sur cette route reçoit `405`.
- Un webhook signé avec deux champs `Host` était accepté avant le correctif (`202`). Le nouveau test processus observe `400 request_malformed` et confirme que l’inbox ne change pas.
- Un webhook signé HTTP/1.1 sans champ `Host` était accepté avant le correctif (`202`). Le nouveau test processus observe désormais `400 request_malformed` sans écriture dans l’inbox. Le contrôle rejette aussi un champ `Host` vide. Il vérifie présence et multiplicité, sans prétendre valider toute la syntaxe d’une autorité.

Les deux refus suivent la règle de [RFC 9112 §3.2](https://www.rfc-editor.org/rfc/rfc9112.html#section-3.2) sur les requêtes HTTP/1.1 sans `Host` et toute requête comportant plusieurs lignes `Host`. Le test est fait contre le vrai binaire et un journal temporaire avec un corps signé synthétique.

## Vérifications

Les suites actuelles passent : receiver processus **11 réussies, 0 échec, 40 ignorées**; HTTP parser **5/5**; handler **12/12**; réconciliation **25/25**; pipeline webhook **2/2**; admission **13/13**; capacité de l’inbox **6/6**; tests unitaires du binaire receiver **4/4**. Les trois scénarios ajoutés passent chacun 1/1. Les 40 tests ignorés exigent Bubblewrap avec `--unshare-net`; ils ne sont pas comptés comme exécutés dans ce lot.

`cargo fmt --all -- --check` et `git diff --check` passent. Le garde de licence passe ses 7 tests. `cargo metadata --offline --no-deps --format-version 1` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `not_assessed`. Aucune licence n’a été choisie; aucune dépendance n’a été ajoutée.

## Portée et limites

La route de découverte ne décrit que le récepteur HTTP local. Elle ne prouve aucune connexion à un Forgejo live ni l’intégration d’AgentOS, Forge Code, RAG, mémoire ou LLM. Le parcours agentique et la comparaison contrôlée restent non démontrés. Le listener est restreint à loopback; aucun service externe ou donnée privée n’a été utilisé. Le validateur `Host` n’est pas un analyseur complet de la syntaxe d’autorité RFC. Le crate reste explicitement non publiable sans décision de licence.

## Artefacts

- Les logs `*-process.log` couvrent les scénarios ciblés; `receiver-process-suite.log` contient aussi les 40 tests ignorés par défaut.
- `*-contract.log` et `webhook-binary-unit.log` conservent les sorties des suites affectées.
- `cargo-license-readback.json`, les logs de garde, `tool-versions.txt`, `commands.txt` et `source-sha256.txt` identifient l’environnement, les métadonnées et les sources.
- `SHA256SUMS` vérifie les artefacts locaux de ce rapport.
