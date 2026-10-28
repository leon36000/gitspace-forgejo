# CONTINUATION-248 — interruption puis reprise pendant `Retry-After`

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, publication, appel externe ou changement de production.

## Scénario

`interrupted_reconcile_during_retry_after_resumes_from_unchanged_journal` fait signer et accepter une livraison d’issue par le vrai receiver Rust. Le vrai CLI démarre avec la livraison et l’API fixture répond `429 Retry-After: 5`. Après écriture complète de cette réponse, le test attend 200 ms, confirme que le CLI reste actif, puis envoie `SIGTERM` au groupe de processus pour interrompre également ses enfants.

Le processus s’arrête par `SIGTERM` dans le budget de 3 s, sans produire de sortie partielle et sans émettre de deuxième GET pendant la fenêtre observée. La fixture ne reçoit qu’un GET. Les octets du journal demeurent identiques et `EventJournal::replay` ne trouve toujours qu’une livraison. Une nouvelle invocation avec le même journal et le même delivery ID lit `200 open`, effectue un GET, retourne `current_state_read` et conserve encore les octets du journal.

La commande ciblée a réussi deux fois sous Bubblewrap `--unshare-net --unshare-pid` : `1 passed; 0 failed; 0 ignored; 152 filtered out`, en 0,90 s puis 0,89 s. Les sorties complètes sont dans `test.log` et `repeat.log`. La répétition utilise le même environnement et n’est pas un échantillon indépendant.

Commande :

```sh
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario interrupted_reconcile_during_retry_after_resumes_from_unchanged_journal
```

## Portée

Le changement ajoute une fixture capable d’observer la réponse 429, le test processus et l’entrée de l’allowlist. Le runtime Rust, `Cargo.toml` et Forgejo ne changent. Le contrôle positif montre la reprise depuis l’inbox durable pour cette livraison après une interruption par signal.

L’API est une fixture loopback, pas Forgejo live. Le test interrompt après 200 ms d’un délai annoncé de 5 s; il ne démontre pas un backoff complet, une panne machine/disque, une reprise distribuée, le nettoyage face à un noyau défaillant, ni une cohérence API atomique. Il n’emploie aucun AgentOS ou LLM et ne mesure aucun gain.

## Licence, vérifications et état projet

Lecture Cargo : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`, marqueur d’export `NOASSERTION`. Les 12 tests de garde de licence passent; aucun choix de licence n’est fait.

`cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent. Aucune suite Rust complète n’a été relancée.

Le parcours reste incomplet : aucun LLM/AgentOS, aucune comparaison Forgejo+agent+CI contrôlée, et aucune preuve d’un principal Forgejo à capacité de fusion uniquement. Les tests et manifestes ici s’ajoutent à l’index courant sans effacer ses anciennes entrées.
