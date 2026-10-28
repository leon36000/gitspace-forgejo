# CONTINUATION-143 — refus au processus d’un `Retry-After` dupliqué

Date : 26 octobre 2026. Worktree observé sur la branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt fortement modifié. Cette tranche ajoute uniquement un test processus et sa sélection par le runner; aucun code de production ne change.

## Parcours observé

Le vrai receiver local accepte une livraison HMAC synthétique `issues` en HTTP 202 et écrit le journal. Le vrai CLI de réconciliation lit cette ligne et interroge une fixture HTTP loopback qui répond HTTP 429 avec deux valeurs du même en-tête, `Retry-After: 0` et `retry-after: 0`. Le CLI retourne code 1 avec un objet structuré `inconclusive/upstream_http_status`, une tentative de lecture, `http_status=429`, aucun `retry_after_ms`, aucune canonical state et le champ `retryable=true` pour une éventuelle décision ultérieure du client. Une observation de 300 ms vérifie qu’aucune seconde requête n’arrive pendant cette fenêtre. L’en-tête ambigu désactive donc le retry automatique du CLI; la fixture ne démontre pas le comportement d’un client externe.

Le test vérifie aussi la route exacte `/api/v1/repos/forge-admin/agent-forge-core/issues/41`, l’absence d’`Authorization`, l’absence du marqueur contenu dans le corps hostile et l’égalité octet pour octet de l’inbox avant/après la lecture. Le scénario compilé hors ligne passe par Bubblewrap `--unshare-net --unshare-pid`: 1 réussi, 0 échec, 0 ignoré, 119 filtrés. Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_disables_retry_for_duplicate_retry_after_headers`; sortie complète dans [focused-run-final.log](focused-run-final.log). Le premier format-check a relevé une seule assertion à replier; la ligne a été formatée puis le run exact a réussi de nouveau. Les deux sorties sont conservées.

## Vérifications et licences

`cargo fmt --all -- --check`, `git diff --check`, `cargo metadata --locked --offline --no-deps`, `scripts/verify_cargo_license_state.py`, ses neuf tests et l’analyse AST du runner passent. Le readback Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. Aucun choix de licence, droit de redistribution ou publication n’est ajouté. Les métadonnées Cargo sont techniquement explicites; la décision juridique du titulaire reste ouverte.

La seule implémentation `AgentRuntimeV1` trouvée sous `src/` et `tests/` est `tests/agent_runtime_contract.rs::FixtureRuntime`; elle n’est pas invoquée ici. Aucun LLM, AgentOS, Forge Code, RAG, mémoire ou service externe n’est utilisé; aucun nouveau coût n’est engagé.

## Portée et limites

Il s’agit d’un scénario synthétique receiver→journal→CLI→HTTP fixture sur un seul événement et un seul ordre de deux valeurs nulles; l’absence de retry n’est observée que pendant 300 ms. Il ne vérifie pas la réaction de Forgejo live, toutes les valeurs ou permutations, ni la décision d’un orchestrateur externe. Le verdict est un échec upstream actionnable, pas une réussite ni une autorisation de fusion. Aucun parcours LLM, benchmark comparatif ou gain de productivité n’est établi; le projet demeure incomplet.

Révision, versions, empreintes des sources et manifestes : [revision.txt](revision.txt), [tool-versions.txt](tool-versions.txt), [source-sha256.txt](source-sha256.txt), [manifest-check.txt](manifest-check.txt) et [global-manifest-check.txt](global-manifest-check.txt). Le journal de reprise et la matrice des refus référencent cette preuve.
