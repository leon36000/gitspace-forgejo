# Reconciliation Forgejo locale — preuve du lot 005

Date : 2026-10-23. Base examinée : commit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Candidat Claude Code 2.1.309 / Opus 5.5 max : `../forgejo-reconcile-opus-worktree`. Les fichiers source intégrés dans ce worktree sont octet pour octet identiques au `src/` du candidat.

## Intégration

Modifiés dans le worktree canonique :

- `src/bin/forgejo-reconcile-local.rs` : découverte `--capabilities` statique/versionnée; version d’API et schémas constants; description des opérations, limites et effets locaux; dérivation de curl `--max-filesize` depuis `MAX_RESPONSE_BYTES`; lecture des fichiers d’entrée sans blocage à l’ouverture des FIFOs avant leur refus par `fstat`.
- `src/webhook.rs`, `src/webhook/reconcile.rs`, `src/lib.rs` : noms d’événements et kinds de journal partagés; ingestion, rejeu et réconciliation gardent exactement `issues` et `pull_request`.
- `scripts/run_webhook_reconcile_cli_e2e.py` : ajout du test de relecture PR.
- `README.md` : ajout de la forme `--capabilities`.

Le document décrit le verrou du journal comme non borné et hors du budget des lectures Forgejo; le préflight utilise `$TMPDIR/agent-forge-merge-locks-<euid>`, un délai de 5 s hors du budget, et la branche fixe `main`. Les écritures Forgejo, les écritures d’état canonique et l’autorisation/tentative de merge sont fausses.

Les tests canoniques protégés n’ont pas été modifiés. Leur empreinte `tests/forgejo_reconcile_capabilities_e2e.rs` reste `47354c1c1b99d09b55c60b340da61169ee81154d629b6985cdfab3e598518aff`. Le candidat contient une copie étendue distincte de 390 lignes (SHA-256 `acfe2d3c038c07b3a5c0381dccb8cdd06d8d096a9f0fd44f8380b986fe2737ba`); elle n’a pas été intégrée parce que le runner épingle les tests du worktree canonique. Ses contrôles d’ensembles exacts, de répertoire sans fichiers, de répétition, d’échappement `./--capabilities` et de FIFO sont donc **non exécutés**. Le correctif `O_NONBLOCK` est compilé et relu, mais le cas FIFO n’est pas démontré à l’exécution.

## Runner fixe

Commande exécutée depuis ce worktree :

```sh
python3 evidence/forgejo-reconcile-sandboxed-tests-001.py ../forgejo-reconcile-opus-worktree
```

Résultat : **7/7**, une réussite par nom exact, dans Bubblewrap 0.9.0, Rust/Cargo 1.95.0, Linux AArch64, Cargo offline/locked, réseau désactivé. Le runner (SHA-256 `9b409abc8ad20b9ace2433ccb4512ef394ea943a458ce8fa59fe402888eedcaf`) a épinglé HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, le manifeste Cargo et les tests/fixtures canoniques, puis a pris le `src/` du candidat. Empreinte du tree réellement testé : `836c87a1298d9ceeb47e3083c1031b1bdb097137a10a44f761b9d3a918d1cec0`.

Tests réussis : capacités en lecture seule; refus des arguments supplémentaires; relecture issue depuis l’inbox et le serveur HTTP privé; relecture PR courante depuis `/pulls/23` malgré un webhook périmé; upstream interdit inconclusif; retries bornés; reprise après interruption. Le test PR exerce le vrai binaire et le vrai récepteur sur un fixture local, sans Forgejo live, création de PR, merge ni LLM.

Journal complet : [005-final](forgejo-reconcile-sandboxed-tests-005-final.log). Une tentative séparée de passer le worktree canonique comme candidat a été refusée par le garde-fou « candidat distinct »; aucun test n’a été exécuté par cette tentative : [trace du refus](forgejo-reconcile-sandboxed-tests-004-same-worktree-refusal.log).

## Format et diff

- Dans le worktree candidat complet, `cargo fmt --all -- --check` passe : [journal candidat](forgejo-reconcile-sandboxed-tests-005-candidate-cargo-fmt.log).
- Après intégration, la même commande sur le worktree canonique échoue uniquement sur le format des deux tests canoniques épinglés, préexistants et laissés intacts : [sortie exacte](forgejo-reconcile-sandboxed-tests-005-canonical-rustfmt-protected-inputs-fail.log). Les quatre fichiers Rust d’implémentation passent `rustfmt --check --edition 2024 --config skip_children=true` : [journal ciblé](forgejo-reconcile-sandboxed-tests-005-implementation-rustfmt.log).
- `git diff --check` passe après la synchronisation et l’ajout du rapport : [résultat final canonique](forgejo-reconcile-sandboxed-tests-005-postdocs-diff-check.log), [candidat](forgejo-reconcile-sandboxed-tests-005-candidate-diff-check.log).

Les tests protégés et le runner sont conservés inchangés; aucune modification de critères, `Cargo.toml` ou `Cargo.lock`.

## Revue indépendante

Claude Sonnet 5.5 max, read-only, sur le candidat final. **Verdict : aucun problème matériel.** La revue a lu le binaire de réconciliation (lignes 1–1415 et plages de tests 1413–1712, 2470–2530 et 2560–2700), `src/webhook.rs` 1–139, `src/webhook/reconcile.rs` 1–325, `src/lib.rs` 425–850 et 1085–1154, `src/forgejo.rs` 940–1155, `src/webhook/http.rs` 190–280, le runner CLI complet et les plages pertinentes des tests protégés. Elle n’a exécuté aucune commande et n’a pas utilisé le résultat du runner comme preuve.

Points mineurs défendus : le refus `--capabilities` a un message de validation spécifique au lieu d’inclure `usage()`; la liste partagée d’événements est liée à des arms de conversion distincts, qui refusent toutefois en mode fermé tout événement sans `ResourceKind`. Le runner CLI secondaire ne vérifie pas lui-même qu’un nom a lancé exactement un test; le runner fixe, lui, montre une réussite `1 passed` pour chacun des sept cas.

## Limites restantes

- Inbox journal non bornée en taille; son verrou peut attendre sans limite. Cela n’étend pas le budget de lecture Forgejo, mais limite le délai total d’une invocation.
- Le verrou de merge dépend de `TMPDIR`; deux valeurs distinctes ne se coordonnent pas et une valeur vide rend le chemin relatif au répertoire courant.
- Le chargeur `--config` conserve une fenêtre TOCTOU entre vérification du lien et ouverture. Le mode du fichier de clé accepte aussi des bits spéciaux en plus de 0600.
- Aucun test exécuté pour le FIFO, le réseau absent de `--capabilities`, le texte d’usage ou la liste d’événements de la réponse processus `unsupported_event`.
- Aucun gain comparatif ni intégration AgentOS, Forge Code, RAG ou mémoire démontré dans ce lot.

Aucun commit, push, publication ou accès à Forgejo externe.
