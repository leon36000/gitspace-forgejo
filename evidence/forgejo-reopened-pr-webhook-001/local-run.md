# FORGEJO-REOPENED-PR-WEBHOOK-001 — réouverture native d’une PR

## Périmètre

Le driver local sait maintenant produire la séquence Forgejo `closed → reopened → closed` pour une PR non fusionnée, lier chaque événement à son UUID de livraison et réconcilier ces notifications avec l’état courant. Le nouveau flag est `--reopen-pull-request`; il refuse la combinaison avec `--merge-pull-request`. Le webhook reste une notification : les cinq événements PR du scénario sont lus en batch par le CLI Rust sur l’état final, fermé et non fusionné.

## Provenance Forgejo

- Source officielle Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; vérification avec `git show <commit>:<chemin>` dans `/tmp/forgejo-v17-base-cas-poc`. Le scratch est globalement modifié par d’autres POC. Les quatre blobs consultés ci-dessous sont identiques à ceux du commit (`git diff --quiet <commit> -- <chemins>` réussit).
- `modules/structs/hook.go:296-297` fixe `HookIssueReOpened` à `reopened`.
- `services/webhook/notifier.go:234-251`, `webhookNotifier.IssueChangeStatus`, envoie `closed` ou `reopened` sous `HookEventPullRequest`.
- `modules/webhook/type.go:68-70`, `HookEventType.Event`, sérialise ce type sous l’en-tête `pull_request`.
- `routers/api/v1/repo/pull.go:749-764` transmet le changement d’état à `issue_service.ChangeStatus`; la même route répond HTTP 201 aux lignes 813-814.
- SHA-256 des blobs lus par `git show <commit>:<chemin>` : `hook.go` `42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12`; `notifier.go` `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f`; `type.go` `ca6a94e1574be77bc4f5da1ee01a3e59147a148342b958577d12544dbd6d820c`; `pull.go` `10586bae76ce9976cfdd44bec2196bd74216e5555a11eae78ca6d41867023dcb`.

## Exécution reproductible

Commande :

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-reopen-local-001-state \
  --evidence-dir evidence/forgejo-reopened-pr-webhook-001 \
  --reopen-pull-request \
  --batch-pull-request-events
```

Le binaire Forgejo annonce `17.0.0+gitea-1.22.0`, construit avec Go `1.26.9`; SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Son commit de build n’est pas établi. Le HEAD produit utilisé pour le build local est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le driver a construit les binaires Rust en mode offline/verrouillé sous Bubblewrap; sortie conservée dans `cargo-build.log`.

## Résultat observé

La PR Forgejo #2 conserve la tête `83dabc792634801ccbf78180db925af57ea5ab25` et la base `317d290579457ea65eac3fe7fed14f9021febbeb`, demeure non fusionnée et finit fermée. Les transitions sont ordonnées dans le journal :

| Séquence | Action | État du payload | UUID |
|---:|---|---|---|
| 5 | `opened` | open | `9b175a36-efa0-437c-a81b-608426a4b849` |
| 6 | commentaire de conversation | open | `e60511ac-9f4f-4420-b6d4-196ae5abf052` |
| 7 | `closed` avant réouverture | closed | `42b7bc99-6f73-42ab-9834-4b230fd1c851` |
| 8 | `reopened` | open | `bc968d97-9b35-40b9-acac-27b53adfb336` |
| 9 | `closed` final | closed | `5157d524-c9e4-4250-895d-288621c284c6` |

Forgejo a enregistré neuf tâches, toutes livrées au receiver avec HTTP 202. Les deux modifications d’état API intermédiaires ont répondu HTTP 201 et leurs GET de readback ont confirmé fermé puis ouvert avant la fermeture finale. Le batch Rust contient les cinq UUID PR ci-dessus : chaque résultat est `current_state_read` sur l’état final `closed`; `upstream_read_attempts=1` est la télémétrie du CLI, pas une capture réseau indépendante. Le receiver a été tué par SIGKILL, relancé avec le même journal, puis a accepté le replay initial comme doublon : 9 enregistrements, 75 778 octets et SHA-256 `20878d2e4d87f8ce112eef96aaafbcd3b9231a2d88dca30a1a1f8447fd5a1c24` avant et après. Le hook est désactivé, Forgejo et le receiver sont arrêtés, et les deux ports sont absents.

## Vérifications et limites

- `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v` : 84 tests réussis; sortie dans `driver-tests.log`. Le test a été observé rouge avant l’implémentation sur le flag absent, l’action `reopened` non admise et le refus de fusionner une PR rouverte.
- `python3 scripts/test_cargo_license_state.py` : 7 tests réussis; état relu dans `cargo-license-state.json` : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` : réussi.
- Le build Rust du driver est réussi et consigné dans `cargo-build.log`; le noyau Rust n’a pas été modifié dans cette tranche. Le manifeste d’exécution `SHA256SUMS` est complété par `SHA256SUMS.addendum` pour les pièces créées ensuite.
- Cette exécution `n=1` utilise un Forgejo jetable sur loopback et le même UID hôte. Elle ne démontre ni CI conventionnelle, ni runtime AgentOS/RAG/mémoire, ni gain de productivité, ni isolation multi-tenant, ni contrôle de fusion contre d’autres writers. Aucun modèle n’a été appelé. Aucun code Forgejo n’a été copié ou déployé.

Les hashes du driver et du test au moment du parcours sont dans `provenance.json`. Les JSON individuels conservent les livraisons, reçus du batch, réconciliation et readbacks.
