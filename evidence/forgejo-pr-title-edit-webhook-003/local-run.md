# FORGEJO-PR-TITLE-EDIT-WEBHOOK-003 — édition native du titre PR

## Objet

Le driver local reconnaît maintenant l’événement Forgejo `pull_request/edited`, contrôle le titre courant et `changes.title.from`, puis lie cette delivery au reconciler Rust. L’événement est inclus dans le batch PR ordonné. Le même run compose la demande et le retrait d’un reviewer avec une fermeture, une réouverture et une fermeture finale; cela vérifie le décompte de toutes les deliveries attendues.

## Source Forgejo vérifiée

- Source officielle Forgejo v17.0.0, commit examiné `0d791421f85ca13a63b665053d81ea765fbbc571`; le scratch `/tmp/forgejo-v17-base-cas-poc` est globalement modifié par les POC, mais son HEAD et les quatre fichiers cités correspondent au commit épinglé (`git diff --quiet <commit> -- <fichiers>` : succès).
- `modules/structs/hook.go:298-299`, `HookIssueEdited`, définit l’action `edited`; SHA-256 du blob extrait par `git show`: `42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12`.
- `services/webhook/notifier.go:190-209`, `webhookNotifier.IssueChangeTitle`, émet pour une PR `HookEventPullRequest`, `Action: HookIssueEdited` et `Changes.Title.From=oldTitle`; SHA-256 `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f`.
- `modules/webhook/type.go:68-70`, `HookEventType.Event`, mappe ce type au header wire `pull_request`; SHA-256 `ca6a94e1574be77bc4f5da1ee01a3e59147a148342b958577d12544dbd6d820c`.
- `routers/api/v1/repo/pull.go:587-665`, `EditPullRequest`, accepte l’édition par API et appelle `issue_service.ChangeTitle`; SHA-256 `10586bae76ce9976cfdd44bec2196bd74216e5555a11eae78ca6d41867023dcb`.

Aucun code Forgejo n’a été copié. Le binaire local est `17.0.0+gitea-1.22.0`, construit avec Go `1.26.9`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build exact est inconnu.

## Commande et résultat

Commande sur un état et un dossier de preuves neufs :

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-pr-title-edit-local-003-state \
  --evidence-dir evidence/forgejo-pr-title-edit-webhook-003 \
  --edit-pull-request-title \
  --remove-review-request \
  --reopen-pull-request \
  --batch-pull-request-events
```

La PR #2 a conservé le head `ea3f4ad01d725d2eb2df3c321a05141b7304d915` et le base `867b1c25147e7009f97321df0e0316de79a9fdc6`. Son titre est passé de `Exercise emitted pull-request webhooks` à `Exercise emitted pull-request webhooks (edited)` (PATCH API 201 et GET de confirmation). La delivery de séquence 9 est `edited`, header `pull_request`, avec le titre précédent exact et le nouveau titre; elle précède les séquences 10 `closed`, 11 `reopened`, 12 `closed`. Les séquences 7 et 8 sont `review_requested` et `review_request_removed`; l’API a répondu 201 puis 204 et les GET ont confirmé l’ajout/retrait.

Les 12 tâches Forgejo du run ont reçu HTTP 202. Les huit livraisons PR ont été réconciliées en une invocation batch; chaque résultat est `current_state_read`, PR fermée/non fusionnée avec les mêmes head/base, et le CLI indique `upstream_read_attempts=1`. Ce compteur demeure une télémétrie applicative, pas une capture réseau indépendante. L’état API final confirme le titre édité, PR fermée, non fusionnée.

Le receiver a été arrêté par SIGKILL avant replay, relancé avec le même journal, puis a accepté le replay comme doublon HTTP 202. Le journal est resté à 12 enregistrements et 109 646 octets; SHA-256 avant/après `52776061d3abf377359d9c682d6fc3cf602bd0e06b32b04f689100df42228c69`. Le hook est désactivé, Forgejo et le receiver sont arrêtés, et les ports sont absents.

## Échec utile et correction du harnais

Le premier run incomplet `-001` a exercé les livraisons, mais s’est arrêté lors du readback final de l’issue. Le run `-002` a conservé un diagnostic expurgé : le contrôle du titre de l’issue comparait une variable locale écrasée par le titre PR édité. Le nom de la variable PR a été rendu distinct, puis le run `-003` a réussi depuis un nouvel état. Les dossiers et reçus des deux échecs sont conservés et ne sont pas présentés comme des parcours réussis.

## Vérifications et limites

- `tests/test_forgejo_emitted_webhook_e2e.py`: 88 tests réussis; quatre nouveaux cas vérifient l’exposition CLI, l’extraction du titre/champ `from` et les refus d’une provenance absente ou inattendue. `py_compile` passe.
- `scripts/test_cargo_license_state.py`: 7 tests réussis. Readback : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`.
- Le build des binaires Rust offline/verrouillé sous Bubblewrap est dans `cargo-build.log`; le code Rust produit n’a pas été modifié dans cette tranche. Les sources exactes compilées sont hachées dans `provenance.json`.
- Run `n=1`, instance Forgejo jetable sur loopback et processus sous le même UID. Le candidat n’exécute pas de code hostile. Aucun LLM, AgentOS, RAG, mémoire ou CI conventionnelle; aucun benchmark, gain, coût modèle ou comportement de production n’est démontré. Ce lot ajoute une capacité d’observation/réconciliation d’événement au harnais local, pas une intégration de production.
- Les `SHA256SUMS` du run couvrent les artefacts générés par le runner; `SHA256SUMS.addendum` couvre le présent rapport et les contrôles ajoutés ensuite.
