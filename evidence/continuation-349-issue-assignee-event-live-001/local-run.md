# CONTINUATION-349 — projection assignees et événements Forgejo live

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`, base de travail `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, push, accès de production ou accès GitLab authentifié n’a eu lieu.

## Sources Forgejo et portée

Le scratch local `/tmp/forgejo-v17-base-cas-poc` pointe sur le commit officiel Forgejo v17.0.0 `0d791421f85ca13a63b665053d81ea765fbbc571`. Le diff ciblé contre HEAD est vide pour les six fichiers ci-dessous; le scratch complet reste modifié par d’autres POC. Lecture de symboles seulement, sans compilation ni tests Go upstream :

| Source | Observation | SHA-256 du fichier |
| --- | --- | --- |
| `routers/api/v1/repo/issue.go::IssueEdit`, lignes 895–959 | `assignees: []` est transmis à `UpdateAssignees`; après relecture, le handler répond 201. | `c88041dc09cdc1097549f00d25fc5a33aefee443c21b5be2ce2e4198b4dcf576` |
| `services/issue/issue.go::UpdateAssignees`, lignes 118–168 | Un tableau vide supprime les assignees courants. | `7d6d951d49a2d8dc597dc7f3861a81d0b976ca12231917e0c8234bdc6483752f` |
| `services/convert/issue.go::ToAPIIssue`, lignes 89–97 | Le champ de sortie est ajouté seulement lorsque la collection lue est non vide. | `a85ec9a4567fb0a87b0af7756f3ddb2366c54375e9ad7471046c9c2fa4fd4776` |
| `modules/structs/issue.go::Issue.Assignees`, lignes 46–62 | `Assignees []*User` porte `json:"assignees"`, sans `omitempty`; une tranche nulle se sérialise en JSON `null`. | `e6cb7890f1d7045b2423465d1e452b6f91c4d71c746957aefc9bade286d10006` |
| `services/webhook/notifier.go::IssueChangeAssignee`, lignes 145–185 | L’émetteur produit l’action `assigned` ou `unassigned` et prépare le hook `issue_assign`. | `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f` |
| `modules/webhook/type.go::HookEventType.Event`, lignes 66–70 | `issue_assign` et les autres catégories issue utilisent le header HTTP `issues`. | `ca6a94e1574be77bc4f5da1ee01a3e59147a148342b958577d12544dbd6d820c` |

## Modification et tests

La projection Rust utilise l’API courante et non les assignees du webhook. Elle trie les IDs positifs distincts, limite la liste à 256 entrées et chaque login à 255 octets UTF-8, conserve `null`/absence comme inconnus et traite `[]` comme vide connu. Toute identité mal formée, dupliquée ou hors limite reste inconclusive. Les capacités CLI annoncent les bornes et ces sémantiques; les schémas single/batch/capabilities sont maintenant 17/21/24.

Le premier test de caractérisation du harnais a échoué pour les deux formes de confusion visées : le validateur rejetait `null` au lieu de le classer inconnu, et ne retournait pas `known_empty` pour `[]`. Après correction, `IssueAssigneeSnapshotTests` passe 3/3. Commandes finales :

- `cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1` — 91 réussis, 0 échec.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=tests python3 -m unittest -q test_forgejo_emitted_webhook_e2e` — 152 réussis, 0 échec.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` — code retour 0.
- `cargo fmt --all -- --check` et `git diff --check` — code retour 0.
- Un `cargo test --offline --locked --all-targets -- --test-threads=1` a été lancé; plusieurs groupes visibles dans la sortie ont réussi, mais la capture n’a pas conservé l’exit code agrégé. Ce run ne sert donc pas de preuve de réussite de toute la suite.

## Parcours Forgejo local

Commande exécutée depuis la racine du worktree :

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root "$PWD" --state-dir /tmp/forgejo-cont-349-issue-assignee-state-003 --evidence-dir "$PWD/evidence/continuation-349-issue-assignee-event-live-001/forgejo-run-003" --change-issue-assignees
```

Forgejo annonce `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, construit avec Go 1.26.9; le commit de build exact n’est pas établi. Le rapport machine [result.json](forgejo-run-003/result.json) a le verdict `passed`, tête source `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, 9 livraisons Forgejo rattachées à 9 tâches HTTP 202, et confirmation de désactivation du hook.

L’ajout d’un collaborateur read-only renvoie HTTP 201 et émet `issues/assigned` séquence 2, avec un tableau d’un assignee dans le payload et le readback d’ajout. L’effacement `PATCH assignees: []` renvoie HTTP 201 et émet une seule livraison distincte `issues/unassigned` séquence 3. Le payload de retrait et le GET API renvoient tous deux `assignees: null`. La base SQLite isolée ne contient ensuite aucune ligne `issue_assignees`, mais cette observation interne n’est pas utilisée comme état canonique. Les deux anciens événements sont réconciliés depuis un GET courant; `canonical_state.assignees` vaut `null` pour chacun et le résultat de l’effacement est `unknown_null_field`, pas `known_empty`. Le replay d’un webhook dupliqué retourne HTTP 202 sans modifier taille, nombre de lignes ou hash du journal.

Arrêts de l’instance et du receiver confirmés, listeners attendus absents, hook désactivé avant arrêt. Le checksum local `forgejo-run-003/SHA256SUMS` couvre les fichiers de sortie du run. Les deux échecs antérieurs sont préservés : `forgejo-run-001` rejetait l’événement `unassigned` dont `assignees` est `null`; `forgejo-run-002` exigeait à tort que le GET courant fournisse `[]`.

## Limites et suite

Cette preuve est un seul parcours local jetable (`n=1`) pour un événement issue; elle ne démontre ni la projection vide connue avec l’API Forgejo v17, ni un parcours LLM/AgentOS, une PR de code, une fusion, une amélioration générale ou un gain comparatif. L’état inconnu est intentionnel. Le dépôt produit n’a toujours pas de licence sélectionnée (`license` et `license_file` absents); aucun code Forgejo ou GitSpace n’a été copié.

Le transfert demandé vers GitLab PC4 n’a pas avancé : le prévol consigné en CONTINUATION-344/347 indique que MagicDNS ne résout pas le nom et que TCP 22/80/443 n’est pas accessible; aucune valeur du courriel n’a été envoyée ou utilisée dans ce lot. La suite distante nécessite une voie réseau joignable et un credential à portée minimale sur `root/gitspace-forgejo`; vérifier d’abord les refs, puis travailler dans une branche dédiée. Continuer les tâches locales indépendantes entre-temps.
