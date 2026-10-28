# CONTINUATION-346 — webhooks live d’étiquettes de PR

Date : 28 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié. Aucun reset, commit, push, changement de remote ou modification de Forgejo amont n’a été fait.

## Résultat

Le harnais live supporte maintenant `pull_request/label_updated` et `pull_request/label_cleared`, ajoute deux livraisons à la vérification des tâches Forgejo, et vérifie la structure de projection des étiquettes renvoyée par le CLI Rust.

La dernière instance jetable, Forgejo `17.0.0+gitea-1.22.0` (binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`), a créé une étiquette de fixture, l’a ajoutée à la PR, puis a supprimé toutes ses étiquettes. L’API a répondu HTTP 200 à l’ajout et 204 à l’effacement. Le journal reçu contient, en ordre, `label_updated` séquence 7 et `label_cleared` séquence 8, avec des identifiants de livraison différents.

Observation importante : le payload de `label_cleared` contient toujours l’étiquette du snapshot antérieur; le GET API après effacement répond avec `labels=[]`. Les deux événements, y compris cette notification périmée, se réconcilient vers la projection courante `{items: [], source: "current_forgejo_resource_api", freshness: "current_api_read", trust: "untrusted_user_content"}`. L’empreinte du webhook ne transforme donc pas son contenu en état canonique. Le résultat complet et l’objet dédié sont dans `forgejo-run-004/result.json`, `webhook-events.json` et `pull-request-label-change-reconciliation.json`.

Le scénario composé a aussi relu neuf tâches webhook Forgejo, chacune livrée au receiver avec HTTP 202, rejoué une livraison après redémarrage du receiver sans changement d’octets du journal, désactivé le hook par API et vérifié l’arrêt des processus et l’absence des listeners suivis. C’est un test local d’intégration des webhooks; il ne fusionne pas la PR et ne valide pas un changement logiciel post-fusion.

## Sources Forgejo et provenance

Le clone officiel `/tmp/forgejo-v17-base-cas-poc` pointe sur le commit exact `0d791421f85ca13a63b665053d81ea765fbbc571`. Lecture ciblée des blobs à ce commit :

- `services/issue/label.go::ClearLabels` (blob `f18dd67a1982f77c184da3a2dd91ef298a77675b`, SHA-256 `881299b59e0a47d955ee8d34dda6f8102b106e7b1fab57c1644f6dfc6cf9fe5e`) appelle `ClearIssueLabels`, puis `IssueClearLabels`.
- `services/webhook/notifier.go::IssueClearLabels` et `IssueChangeLabels` (blob `5aea521a0f2cb5d7513c98e9805d10e6a6b2901b`, SHA-256 `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f`) émettent les snapshots de PR et les actions correspondantes.
- `modules/structs/pull.go::PullRequest.Labels` (blob `1ce7550e197752d5065283bab01bc7da98dfb901`, SHA-256 `1472f689d9e80da512cc8edafa5d647daafccdf060dfba0d4a74d3be5cfe06e3`) expose le champ wire `labels`.

Le binaire live confirme sa version déclarée et son hash, mais ces éléments n’établissent pas le commit précis dont il a été compilé. L’examen amont est limité à trois fichiers, aux symboles/callsites ci-dessus et à un tag; aucun test Go de Forgejo, audit exhaustif du dépôt, autre version ou autre backend n’est couvert. Aucun code Forgejo n’est copié dans le produit.

## Diagnostic et correction

Le premier run (`forgejo-run/`) a échoué au stage `deliver_pull_request_label_cleared_event`: le harnais exigeait un payload de clear vide. Le journal et le readback API montraient au contraire un payload répétant l’étiquette précédente, tandis que le GET API confirmait la suppression. La validation appelle maintenant `validate_live_pull_request_label_clear_snapshot`: elle exige que le payload du clear corresponde au snapshot de l’événement d’ajout, que ce snapshot contienne l’étiquette de la fixture, et que la lecture API après effacement soit vide.

Deux erreurs distinctes du harnais ont été trouvées et corrigées ensuite :

1. `forgejo-run-002/` a montré que la liste d’événements passée à `validate_hook_task_bindings` omettait les deux PR labels. Forgejo avait enregistré neuf tâches et le journal neuf livraisons, mais la liste attendue en contenait sept. `pull_request_label_events` est maintenant inclus dans les événements à relier.
2. `forgejo-run-003/` a montré que l’assertion comparait `canonical_state.labels` à une liste. Le contrat Rust sérialise réellement une enveloppe avec `items`, `source`, `freshness` et `trust`. `is_current_empty_label_projection` exige cette forme exacte et l’origine API courante.

Les tests ciblés avaient précédemment produit un rouge lorsque le parser refusait les actions natives; les logs rouge/vert sont conservés dans ce dossier. Les tests unitaires nouveaux acceptent le payload de clear périmé seulement s’il correspond au snapshot précédent et à un readback API vide; ils refusent un snapshot différent et une projection vide sans provenance API.

## Vérifications

- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e.PullRequestLabelSnapshotTests tests.test_forgejo_emitted_webhook_e2e.HookTaskBindingTests -v` — 8/8.
- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v` — 138/138.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` — succès.
- `git diff --check` — succès.
- Parcours Forgejo v17 local : `python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-cont-346-pr-label-e2e-004 --evidence-dir "$PWD/evidence/continuation-346-pr-label-event-live-001/forgejo-run-004" --change-pull-request-labels` — code 0, verdict `passed`.

Les trois premiers runs sont conservés comme échecs diagnostiques dans `forgejo-run/`, `forgejo-run-002/` et `forgejo-run-003/`; le dernier est le run vert. Aucun test Rust complet n’a été relancé : le runtime Rust n’a pas changé dans ce lot. Le full suite Rust documenté par CONTINUATION-345 reste une preuve historique, distincte.

## Limites et prochaine étape

`n=1` instance locale pour le comportement observé. Ce résultat établit l’émission live et la réconciliation de ces deux transitions pour cette version déclarée du binaire; il ne prouve pas toutes les actions PR, tous les fournisseurs, toutes les courses ou un gain général. Pas d’AgentOS/LLM, pas de GitLab réel, pas de modification de production, pas de merge de cette PR et pas de validation du comportement d’une modification après fusion.

Le dépôt GitLab PC4 reste inaccessible depuis ce poste au dernier recontrôle : ping Tailscale de `pc4` réussi, résolution MagicDNS en échec, connexions TCP 22 et 443 en échec. Le relevé des courriels et le périmètre des identifiants trouvés sont documentés dans [CONTINUATION-344](../continuation-344-gitlab-credential-scope-and-network-001/local-run.md); le jeton cité dans ce relevé concerne un autre projet, et le mot de passe initial `root` impose une rotation au premier login. Aucun identifiant n’a été utilisé, transmis ou modifié dans ce lot. La prochaine étape locale est un autre cas non couvert de la matrice; la migration distante attend que le service GitLab et un credential limité au dépôt cible soient utilisables sans rotation non autorisée.
