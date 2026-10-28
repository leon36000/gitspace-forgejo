# CONTINUATION-388 — redelivery corrigée du même ID après redémarrage

Date : 28 octobre 2026. Branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun code runtime n’a été modifié.

## Scénario et résultat

Le test processus démarre le vrai `forgejo-webhook-local` avec un journal temporaire. Un `issue_comment` signé sans `is_pull` reçoit HTTP 400, `event_payload_invalid`, `retryable=false`; après arrêt du premier receiver, la relecture du journal est vide. Un second receiver ouvre le même chemin. Le payload corrigé réutilise le même delivery ID, fournit `is_pull=true` et un objet `pull_request` ayant le même numéro que `issue`. Il reçoit HTTP 202, `accepted=true`, `duplicate=false`, `sequence=1`. Après arrêt, le replay ne contient qu’un événement, dont la clé `forgejo-delivery-v1:<id>` correspond au même delivery ID et dont le payload est la version corrigée.

La commande est dans `verification-commands.txt`; le runner exécute le test sous Bubblewrap `--unshare-net --unshare-pid`. Résultat : 1 réussi, 0 échec, 183 filtrés. Rustfmt et `git diff --check` passent.

Le premier essai de compilation du test utilisait le champ inexistant `JournalEvent.delivery_id`. La structure expose `idempotency_key`; l’assertion est corrigée et la relance passe. Cette sortie est conservée dans `receiver-restart-process-e2e-compile-failure.log` pour distinguer ce défaut du test d’un défaut produit.

## Limites

Le résultat montre seulement qu’un ID rejeté avant journalisation peut être réutilisé par un payload corrigé après redémarrage. Il ne teste pas la récupération d’une livraison déjà acceptée, SIGKILL pendant un append, un événement émis par Forgejo live, la mission LLM/AgentOS ou une fusion. Le scénario reprend la forme contractuelle Forgejo v17 mais utilise des données de fixture.
