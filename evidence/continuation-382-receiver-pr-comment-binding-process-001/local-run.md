# CONTINUATION-382 — refus d’un commentaire PR mal lié dans le processus receiver

Date : 28 octobre 2026. Worktree local HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0.

## Scénario

Le receiver compilé est lancé comme processus séparé par le test Rust. Il écoute sur loopback, vérifie une signature HMAC de fixture et écrit dans un journal JSONL temporaire. Le premier payload a une action valide (created), issue 14, comment 22 et is_pull=true, mais aucun objet pull_request.

Le receiver répond HTTP 400 event_payload_invalid, retryable=false et indique que le corps rejeté n’a pas été journalisé. Le readback du journal est vide : le delivery ID n’a pas été réservé. Le test ajoute ensuite pull_request.number=14 et renvoie le même ID; le processus retourne HTTP 202, accepted=true, duplicate=false, sequence=1. Après arrêt propre et relecture du journal, une seule ligne existe et conserve les deux marqueurs is_pull=true et pull_request.number=14.

Cela démontre au niveau du vrai receiver-processus que le refus sémantique arrive avant l’écriture durable et que la correction peut être redélivrée avec le même identifiant. La protection existait dans le code avant ce test; aucun code de production n’a été modifié.

## Preuves

- Scénario ciblé actual_receiver_refuses_pull_comment_without_pull_request_then_accepts_same_delivery : 1/1.
- Cible complète webhook_receiver_process_e2e : 42 réussis, 0 échec, 139 ignorés. Les tests ignorés ne sont pas couverts.
- cargo fmt --all -- --check et git diff --check : réussis.
- scripts/test_cargo_license_state.py : 12/12.
- Cargo metadata offline : license=null, license_file=null, publish=[].

L’assertion receiver précédente CONTINUATION-381 couvre les contrats handler/reconciler; ce lot ajoute un processus réel et un retry corrigé, pas un résultat d’API simulé dans le même processus. Forgejo upstream v17.0.0 au commit 0d791421f85ca13a63b665053d81ea765fbbc571 définit IssueCommentPayload dans modules/structs/hook.go:203–213, avec PullRequest optionnel et IsPull booléen.

## Limites

Le secret HMAC est synthétique. Le service ne quitte pas le loopback local, mais n’est pas enveloppé ici dans un namespace réseau privé. Aucun webhook natif Forgejo n’est émis; aucun CLI de réconciliation ne traite un journal contenant ce payload refusé, puisqu’il n’y a pas d’événement durable; le test ne démontre pas AgentOS, LLM, PR, fusion ou gain. Les 139 scénarios ignorés gardent leur statut. Ne pas assimiler ce test à une vérification complète du parcours demande→fusion.

Suite : identifier un écart de processus encore ouvert pour le reconciler CLI ou un événement Forgejo qui n’a pas de preuve de readback courant, puis ajouter une seule tranche sans recopier les tests existants.
