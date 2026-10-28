# CONTINUATION-367 — null explicite d’assignees d’Issue connu vide

Date : 28 octobre 2026. Travail limité au worktree local `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, sans commit, push, publication ou modification de production.

## Hypothèse et preuve source

Forgejo officiel v17.0.0 est audité au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. L’inventaire source CONT-349 indique que `services/convert/issue.go::ToAPIIssue` copie la tranche ORM des assignees seulement lorsqu’elle n’est pas vide, tandis que `modules/structs/issue.go::Issue.Assignees` est sérialisé sans `omitempty`. Le run réel `forgejo-run-003` a soumis le retrait d’assignees et a relu `assignees: null` depuis l’API Issue. Ces sources justifient l’inférence étroite qu’un null explicite sur la réponse API Issue signifie la liste vide; elles ne démontrent pas la sémantique null d’une réponse PR. Références, lignes et portée de l’examen : [CONT-349](../continuation-349-issue-assignee-event-live-001/local-run.md) et [source-inventory.md](../../docs/source-inventory.md).

## Changement

- `src/webhook/reconcile.rs::current_work_item_assignees` distingue désormais l’absence du champ (`None`) du null explicite : Issue/parent Issue → projection `known_empty`; Pull Request/parent PR → inconnu; tableau vide → `known_empty`. Les IDs et logins projetés restent bornés/triés et de confiance `untrusted_forgejo_identity`.
- La découverte de capacités annonce `schema_version=26`, `forgejo-reconcile-capabilities-v26`, `missing_field=null_unknown`, et la sémantique `null_by_resource`.
- Le driver live Python normalise la réponse API Issue à partir de l’objet entier. Il préserve ainsi champ absent versus null; la normalisation des payloads webhook et celle des réponses PR ne changent pas.
- Aucune copie de code Forgejo/GitSpace et aucune décision de licence produit.

## Test-first et vérifications

Les trois échecs observés avant implémentation et la précaution sur les journaux mal nommés sont dans [test-first-observations.md](./test-first-observations.md).

Après correction :

- `cargo test --offline --locked --all-targets -- --test-threads=1` — 38 groupes, **743 réussis, 0 échec, 169 ignorés**; sortie [green-all-targets-final.log](./green-all-targets-final.log).
- `cargo test --offline --locked --test webhook_reconcile_contract` — 96/96; les tests couvrent Issue null connu vide, champ absent inconnu, PR null inconnu, parent commentaire issue null connu vide et parent commentaire PR null inconnu.
- `cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles` et `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities` — passent.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=tests python3 -m unittest -q test_forgejo_emitted_webhook_e2e` — 161 tests, succès.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` — succès. `cargo metadata --offline --locked --no-deps --format-version 1` confirme pour `agent-forge-core 0.1.0` : `license=null`, `license_file=null`, `publish=[]`; aucune licence n’est sélectionnée.

Les commandes et reçus post-correctif sont conservés dans ce dossier. La garde Cargo ne tranche pas les droits juridiques ni la redistribution.

## Limites et GitLab PC4

Il n’y a pas eu de nouveau parcours Forgejo live : une requête anonyme à `127.0.0.1:3130/api/v1/version` a échoué par `ConnectionRefused`. Les tests de ce lot sont des contrats Rust/Python; le seul readback live utilisé pour l’inférence est CONT-349, antérieur au correctif. Aucun parcours LLM/AgentOS, PR ou fusion n’est démontré par ce lot.

L’utilisateur a demandé de transférer le travail vers `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` et a indiqué que l’accès est dans son courriel. La recherche Gmail explicitement demandée a retrouvé un courriel de jeton; le checkpoint CONT-344/336 indique que le PAT trouvé a déjà été exposé et vise un autre projet. Il n’a pas été utilisé et aucune valeur de secret n’est incluse ici. PC4 apparaît en ligne dans Tailscale mais MagicDNS ne résout pas dans cet environnement; TCP 22 et 443 expirent vers le peer. Aucun accès Git distant n’a été établi.

Le worktree est fortement modifié (19 entrées suivies dans l’index de statut, sans staging ni suppression, 1 755 entrées non suivies au dernier comptage). Un audit d’export est nécessaire avant transfert; les caches et secrets synthétiques sous `.runtime` ne doivent pas être copiés. Action humaine nécessaire : révoquer/renouveler le PAT exposé et rétablir la connectivité PC6→PC4. Le projet reste incomplet.
