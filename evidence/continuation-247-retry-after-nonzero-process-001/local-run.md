# CONTINUATION-247 — `Retry-After` positif observé au processus

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, publication, service externe ou instance de production.

## Scénario et résultat

Le scénario `actual_reconcile_cli_process_waits_for_nonzero_in_budget_retry_after` fait accepter une livraison signée HMAC par le vrai receiver Rust, puis lance le vrai CLI de réconciliation. Sa fixture API répond `429 Retry-After: 1`, puis `200 open`. La fixture mesure l’intervalle entre la fin d’écriture de la réponse 429 et l’acceptation de la connexion retry; le test exige au moins une seconde et une borne totale de 10 s. Il vérifie aussi exactement deux GET vers la ressource issue, `upstream_read_attempts=2`, `current_state_read`, aucun header `Authorization`, aucune fuite du corps 429 et aucune modification de l’inbox.

Après ajout de la mesure fixture entre la réponse et la connexion suivante, le scénario ciblé a réussi deux fois (`1 passed`, `0 failed`, `151 filtered out`; durée 1,38 s puis 1,37 s). Les logs des deux runs finaux sont `final-run-001.log` et `test.log`. Un probe préliminaire moins précis est conservé dans `initial-timing-probe-before-refinement.log` mais n’est pas compté comme preuve. Le runner compile hors ligne puis exécute sous Bubblewrap `--unshare-net --unshare-pid`, avec une fixture loopback privée.

Commande :

```sh
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_waits_for_nonzero_in_budget_retry_after
```

## Différence et portée

Ce lot ajoute une fixture processus qui mesure l’intervalle réponse/retry, le test et son nom dans l’allowlist du runner. Aucun code runtime Rust, `Cargo.toml`, credential ou intégration Forgejo ne change. Le run clôt une lacune explicitement notée pour CONT-243, qui n’avait testé que le délai nul.

La réponse est une fixture locale, pas une instance Forgejo ni un émetteur externe. La borne est un minimum de 900 ms pour `Retry-After: 1`, pas une mesure de latence exacte ou une garantie de succès; `n=1`, même binaire/configuration pour les deux répétitions, pas une population indépendante. Aucune charge ni LLM/AgentOS.

## État de licence et checks

Lecture Cargo fraîche : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, marqueur SPDX exporté `NOASSERTION`; les 12 tests de garde passent. Cela conserve l’absence de décision et n’évalue pas les droits de redistribution.

`cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent. Aucune suite globale Rust n’a été relancée.

La frontière de fusion demeure distincte : le processus gate a une allowlist de routes côté client et le profil d’essai passe un token ayant `read:user`, `write:repository`, `write:issue`; ceci ne démontre pas un principal Forgejo à capacité de fusion uniquement ni l’exclusion des autres writers. Aucun nouveau test de cette frontière n’est revendiqué ici. Les sources/symboles, hashes, commandes, versions et sorties sont listés dans les fichiers voisins; le checkpoint et la matrice sont actualisés.

Le projet demeure incomplet : AgentOS/LLM autorisé, comparaison appariée et valeur généralisable, modèle de licences/droits, refus cumulatifs et séparation effective des autorités restent ouverts.
