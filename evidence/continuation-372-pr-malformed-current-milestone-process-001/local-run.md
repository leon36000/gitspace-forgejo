# CONTINUATION-372 — refus d’un milestone PR API mal formé au niveau processus

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste modifié; 19 chemins suivis et de nombreux chemins non suivis sont préservés. Aucun reset, staging, commit, suppression, fetch ou push n’a eu lieu. Voir `environment.txt` pour les versions et le statut condensé; `source-sha256.json` lie les sources et documents concernés.

## Lacune et contrat

Le contrat Rust `malformed_current_pull_request_milestone_refuses_canonical_snapshot` couvre une valeur invalide, mais aucun test processus ciblé ne liait encore ce refus à un événement PR reçu, au journal et au CLI. Ce lot ajoute `actual_reconcile_cli_process_refuses_malformed_current_pull_request_milestone` et l’inscrit dans l’allowlist de `scripts/run_webhook_reconcile_cli_e2e.py`. Le code runtime `current_work_item_milestone` dans `src/webhook/reconcile.rs` n’a pas été modifié.

La fixture envoie au receiver un webhook signé `pull_request/milestoned`, valide, avec un milestone historique ID 99. Le receiver renvoie HTTP 202, séquence 1; après fermeture du receiver, le CLI réel lit une réponse API PR courante dont `milestone.id` vaut 0. Le test exige une sortie structurée `inconclusive/invalid_current_state`, `retryable=false`, `details.field=milestone`, un seul readback, aucun `canonical_state`, aucun header Authorization, aucun texte du milestone historique dans la sortie et un journal byte-identical.

## Test et vérification

Avant l’entrée de scénario, le runner refuse le nom avec argparse exit 2 (`runner-allowlist-red.log`). Après ajout, la commande ci-dessous passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid`; la compilation est offline et le runner masque `/home` et `/tmp`, avec limites de ressources. La sortie complète contient la commande sandboxée et le résultat exact (`process-e2e.log`). Le test reste `#[ignore]` dans la suite normale et s’exécute via ce runner.

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_malformed_current_pull_request_milestone
cargo fmt --all -- --check
python3 scripts/test_cargo_license_state.py
python3 scripts/verify_cargo_license_state.py
git diff --check
```

Résultats : processus 1/1 avec 176 tests filtrés; rustfmt, `py_compile` et `git diff --check` passent; scan `rstrip()` complet du fichier de test et du runner ne détecte aucun espace final. Les 12 tests de garde licence passent. `cargo metadata` reste `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`. Aucun choix de licence, changement Cargo ou ajout de dépendance.

## Limites et suite

Les événements et le GET sont synthétiques/loopback; un seul représentant malformé est composé, tandis que le contrat unitaire couvre les autres formes. Ce résultat ne démontre pas une émission native Forgejo, une intégration AgentOS/modèle, une fusion, un comportement post-fusion ou un gain comparatif. Aucun AgentOS ou LLM n’a été invoqué; la fixture est explicitement un substitut pour la seule frontière webhook/API/CLI. Le seul trafic réseau était le loopback du test.

Prochaine lacune utile : exercer au niveau processus la différence entre `milestone` absent (`unknown`) et `milestone: null` (`unassigned`), sans rejouer les parcours CONT-370/371. Le transfert PC4 n’a pas été revalidé dans ce lot.
