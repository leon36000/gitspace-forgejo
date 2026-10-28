# CONTINUATION-305 — numéro racine contradictoire dans le processus receiver pull_request

Date : 27 octobre 2026. Worktree produit basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun code de production n’est changé.

## Résultat

Le test `actual_receiver_rejects_conflicting_pull_request_root_number_before_append_and_accepts_corrected_retry` lance le vrai binaire `forgejo-webhook-local` sur loopback, avec un secret HMAC synthétique et un journal temporaire.

1. Une livraison signée `pull_request/opened`, portant `number=42` et `pull_request.number=41`, reçoit HTTP 400, `accepted=false`, `event_payload_invalid` et `retryable=false`.
2. Le journal reste vide : le refus ne réserve pas le delivery ID.
3. Le test corrige le numéro racine à 41, réutilise ce même delivery ID et reçoit HTTP 202, `duplicate=false`, `sequence=1`.
4. Après arrêt du receiver, le readback trouve exactement une ligne dont le numéro racine et `pull_request.number` valent 41.

Commande exacte :

```text
cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_rejects_conflicting_pull_request_root_number_before_append_and_accepts_corrected_retry -- --exact
```

Résultat : 1 réussi, 0 échec, 0 ignoré, 166 filtrés; le test prend 0,07 s. La sortie est dans `process-test.log`; le SHA-256 du binaire receiver et du harnais sont dans `toolchain.txt`. `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` passent. Le résultat de la garde conserve `license=null`, `license_file=null`, `publish=[]`, droits `not_assessed` et `NOASSERTION`.

## Portée

Ceci complète le test processus CONT-303 pour `issues`; le contrat/parser CONT-302 couvrait déjà les deux formes de payload. Le scénario est local et synthétique : il n’utilise pas d’instance Forgejo, de réconciliation, de PR réelle, de modèle LLM/AgentOS ni de credential réel. Il prouve le comportement du receiver Rust et de son journal temporaire, pas la vérité d’un webhook amont ou l’identité de son producteur. Aucun code Forgejo/GitSpace, service externe ou coût n’est utilisé.

La suite doit choisir dans la matrice une autre lacune concrète non prouvée; ne pas rejouer les parcours sémantique ou Forgejo déjà conservés. Le runtime AgentOS/LLM autorisé, la comparaison conventionnelle appariée et la licence GitSpace restent non résolus; la mission est incomplète.
