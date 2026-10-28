# Webhook Forgejo natif de synchronisation de PR, 25 octobre 2026

## Résultat

Le parcours local Forgejo v17.0.0 crée une PR privée, avance réellement sa branche par l’API Contents, reçoit l’événement natif de synchronisation, le journalise, le réconcilie contre l’état courant de Forgejo, fusionne la tête avancée, puis vérifie le commit de `main` et le contenu exact du fichier fusionné. Résultat : `passed`.

La tâche Forgejo est `pull_request_sync`, mais son en-tête HTTP est `pull_request` et son payload porte `action=synchronized`. Le premier essai du harnais avait supposé à tort que le nom de la tâche était aussi l’en-tête; il a échoué à cette assertion après avoir reçu les six premiers événements. Le run raté est conservé dans [forgejo-pr-sync-merge-001](../forgejo-pr-sync-merge-001/failure.json). La relecture du code v17 a confirmé le mapping corrigé avant le run final.

## Sources et portée

Forgejo officiel v17.0.0 est audité au commit `0d791421f85ca13a63b665053d81ea765fbbc571` dans `/tmp/forgejo-v17-base-cas-poc` (scratch globalement modifié; les fichiers cités sont inchangés). `modules/webhook/type.go::HookEventType.Event` mappe `HookEventPullRequestSync` vers `pull_request`; `services/webhook/notifier.go::PullRequestSynchronized` émet l’action `synchronized`; `modules/structs/hook.go::HookIssueSynchronized` définit cette valeur. Les empreintes des trois fichiers sont conservées dans `docs/source-inventory.md`.

## Commande et observation

Commande exécutée depuis le worktree local :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-pr-sync-merge-002 --evidence-dir "$PWD/evidence/forgejo-pr-sync-merge-002" --synchronize-pull-request --merge-pull-request
```

Code retour `0`. Forgejo annonce `17.0.0+gitea-1.22.0`; SHA-256 du binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le binaire local annonce Go `1.26.9`. Le build local Rust utilise Cargo `1.95.0`, rustc `1.95.0`, Bubblewrap `0.9.0`, `--unshare-net`, `--unshare-pid`, environnement nettoyé, sources en lecture seule, et `cargo build --offline --locked`.

Le run journalise 8 événements et valide 8 tâches Forgejo HTTP 202 : quatre notifications d’issue, PR `opened`, PR `synchronized`, commentaire `issue_comment`, puis PR `closed`. La tête passe de `f52ae90964e24e7f9971fcdc012c88c808b39d65` à `3cc4049c07a8c1c0d3c8adbf37ce260a19cd6d74`. Le commit de fusion et la tête relue de `main` valent `8bd8b89932815e528a91f300e8d69f63a27d448a`; le contenu lu sur `main` égale exactement les octets après synchronisation. La réconciliation de l’événement `synchronized` retourne l’état courant fermé/fusionné et les SHA Forgejo actuels.

Après SIGKILL du récepteur, son redémarrage conserve le journal; la redélivrance du premier événement est acceptée comme doublon, sans changement de taille ni de SHA-256 du journal (`250f1f89723297ba894c14d63b3fe313c02fff74e15dff5bf0f3d9e34a8fc681`). La désactivation du webhook, l’arrêt des processus et l’absence des listeners attendus sont confirmés. Les JSON de résultat, readbacks, tâches Forgejo, rejeu, provenance et nettoyage sont archivés dans ce dossier.

## Vérifications

- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v` : 61 réussis.
- `cargo test --all-targets -- --test-threads=1` : 521 réussis, 0 échec, 63 ignorés, 34 groupes.
- `python3 scripts/verify_cargo_license_state.py` : vérifié, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`; `python3 -m unittest scripts.test_cargo_license_state -v` : 7 réussis.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` : code retour `0`.
- `sha256sum -c evidence/forgejo-pr-sync-merge-002/SHA256SUMS` : tous les fichiers couverts sont valides; ce manifeste local est actualisé après ajout du présent rapport et des logs de tests.

## Limites

Forgejo et le récepteur s’exécutent dans une instance jetable privée sur loopback; le runner modifie le code de candidat via l’API Forgejo. Aucun modèle, LLM, AgentOS, code hostile, CI réelle ou vérificateur indépendant n’intervient dans ce parcours. La fusion est une opération de test explicite, pas la démonstration de l’autorité de fusion du produit. Ce succès vérifie le contrat de webhook et la réconciliation d’une tête qui avance; il n’établit aucun gain général, coût, ni complétion de la mission.
