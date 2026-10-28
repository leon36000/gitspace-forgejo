# CONTINUATION-157 — action exploitable pour un framing HTTP invalide

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; aucune modification antérieure n’a été remplacée, supprimée ou commitée.

## Changement et preuve

Le test processus existant `tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_case_variant_conflicting_content_lengths_before_append` démontrait le refus HTTP 400, l’absence d’append et l’acceptation ultérieure du même ID corrigé. Il ne vérifiait pas si l’erreur expliquait comment corriger le cadrage. J’ai d’abord ajouté l’assertion sur `error.action`; le run rouge a trouvé l’ancien texte « send one complete HTTP request with Content-Length ». La sortie observée est conservée dans `red-phase-observed.log`.

Le champ `action` de `request_malformed` dans `src/bin/forgejo-webhook-local.rs` précise maintenant : au plus un `Content-Length`, aucun `Transfer-Encoding`, et un `Host` pour HTTP/1.1. Le même test processus passe ensuite : **1 réussi, 0 échec, 122 filtrés**, 0,06 s dans le run enregistré. Il confirme le nouveau texte, le code `request_malformed`, `retryable=false`, l’inbox byte-for-byte inchangée après refus, puis l’acceptation de la requête corrigée avec le même ID à la séquence 1.

Le statut HTTP et la décision de refus n’ont pas changé. Seule l’action de correction présentée par ce chemin d’erreur a changé. Le test utilise le vrai binaire/receiver local sur loopback et un événement synthétique signé; il n’appelle aucune API externe.

## Vérifications finales

- Le test ciblé ci-dessus : **réussi**; sortie dans `process-test.log`.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs src/bin/forgejo-webhook-local.rs` : réussi.
- `git diff --check` : réussi.
- `python3 scripts/verify_cargo_license_state.py` : réussi; `license=null`, `no_license_selected`, `NOASSERTION`, droits de redistribution `not_assessed`.
- Les préfixes antérieurs des manifestes local, worktree et projet sont enregistrés avant ajout et vérifiés par le script append-only de ce dossier.

Environnement observé : rustc 1.95.0, Cargo 1.95.0, Python 3.12.3, Linux aarch64. HEAD et état des fichiers suivis avant les nouvelles pièces de preuve : `toolchain-and-head.txt` (les nombreux fichiers non suivis préexistants ne figurent pas dans ce relevé). Les empreintes des sources et documents : `source-sha256.txt`.

## Limites

Un seul test processus, une seule forme de framing HTTP et un seul changement de message. Pas de reverse proxy, désynchronisation entre parseurs, Forgejo live, LLM/AgentOS, comparaison conventionnelle ou mesure d’usage réel. Ce lot améliore le diagnostic d’un refus local; il n’établit ni la sécurité universelle ni un gain du produit. La mission reste incomplète.
