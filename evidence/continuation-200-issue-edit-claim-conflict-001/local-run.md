# CONTINUATION-200 — issue éditée après revendication d’allocation

## Résultat

Le test processus ciblé `actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation` passe **1/1** (`3 filtered out`). Il exerce le receiver Rust et le CLI `forgejo-mission-local` dans le harnais Bubblewrap. Après la revendication initiale de l’allocation, un retry identique reste `already_claimed`. Une livraison signée `issues/edited`, avec un nouvel UUID et un payload périmé, déclenche ensuite une lecture de l’API loopback de fixture, qui retourne un titre, un corps et `updated_at` modifiés. Le CLI répond `inconclusive/execution_idempotency_conflict`; le journal de mission et le handoff restent identiques octet pour octet. Aucune nouvelle génération n’est revendiquée automatiquement.

La commande reproductible est :

```sh
python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation
```

La sortie complète est dans `forgejo-mission-delivery-process.log`. Le runner compile hors réseau (`CARGO_NET_OFFLINE=true`) puis exécute le test avec Bubblewrap `--unshare-net --unshare-pid`; versions et empreintes de la source sont dans `toolchain-and-source-hashes.json`. `cargo fmt --all -- --check` et `git diff --check` passent; leurs sorties sont vides dans `rustfmt.log` et `diff-check.log`.

## Portée et limites

C’est un test d’intégration processus du receiver et du CLI produits, avec une API Forgejo synthétique sur loopback; le test n’utilise pas une instance Forgejo ni un émetteur natif live. `n=1`, sur un seul issue et une seule allocation. Il protège contre l’admission silencieuse d’un nouveau contexte sous un claim déjà attribué; une autorité hôte doit traiter explicitement le conflit avant de créer une nouvelle génération. Il ne démontre ni parcours agent/LLM, AgentOS, merge, benchmark comparatif, ni effet sur les performances réelles. Aucun accès externe, coût modèle, secret ou environnement de production n’a été utilisé.

Le test est marqué `#[ignore]` et a été invoqué explicitement par le runner dédié. HEAD était `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree est fortement modifié et aucun changement préexistant n’a été réinitialisé, nettoyé ou commité.
