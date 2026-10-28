# CONTINUATION-018 — rejet de corps surdimensionné au vrai processus receiver

Date de travail : 25 octobre 2026. Produit `agent-forge-core` 0.1.0, checkout HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree préexistant très modifié. Le test ajouté est dans `tests/webhook_receiver_process_e2e.rs`; les empreintes finales sont listées dans `source-sha256.txt`.

## Lacune observée

La limite `MAX_WEBHOOK_BODY_BYTES` et son refus `HttpRequestError::BodyTooLarge` étaient couverts par les contrats du parseur et du handler, mais aucun test n’exerçait ce refus via le vrai binaire receiver. La nouvelle caractérisation envoie un vrai `POST /forgejo/webhook` avec `Content-Length = MAX_WEBHOOK_BODY_BYTES + 1`, puis ferme l’écriture sans transmettre aucun octet du corps ni de signature. Elle observe HTTP 413 / `body_too_large`, `retryable=false`, l’inbox inchangée et un replay vide.

Le serveur a donc interprété l’en-tête de longueur et refusé le corps avant d’attendre ses octets ou d’appeler la vérification webhook. Aucune modification de production n’a été nécessaire : le comportement existant est désormais prouvé au niveau du processus.

## Vérifications

- Le test processus ciblé `actual_receiver_rejects_oversized_content_length_without_waiting_for_the_body` passe 1/1 (`targeted.log`, code 0).
- `cargo test --locked --offline --all-targets` : **539 réussis, 0 échec, 76 ignorés, 34 groupes**, code 0 (`all-targets.log`, `all-targets.exit`, `all-targets-summary.txt`).
- `cargo fmt --all -- --check` et `git diff --check` passent.
- Le garde de licence Cargo passe 9/9; le readback confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed` et `NOASSERTION`. Aucun choix de licence ni dépendance n’a été ajouté.

## Portée et limites

Le scénario est un processus receiver local sur loopback, fichier inbox temporaire et même UID; il ne teste pas une instance Forgejo live ni l’isolation OS. Le corps est volontairement absent : le test prouve le refus fondé sur la longueur déclarée, pas l’usage mémoire avec plusieurs connexions ni un transfert complet de 1 Mio. Aucun modèle, AgentOS, accès externe, coût ou donnée privée n’est utilisé. Le parcours LLM, le benchmark contrôlé et le projet global restent incomplets.

Suite : confronter la matrice des refus composés à la couverture déjà enregistrée et choisir un comportement réellement absent avant le prochain changement. Les hypothèses de précondition des writers Forgejo et la récupération LFS restent dans leur scratch non adopté.
