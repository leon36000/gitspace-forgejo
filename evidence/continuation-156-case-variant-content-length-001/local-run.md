# CONTINUATION-156 — Content-Length contradictoires avec casse variable

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. État Git fortement modifié; aucune modification antérieure supprimée ou commitée.

## Scénario et résultat

Le nouveau test processus `actual_receiver_rejects_case_variant_conflicting_content_lengths_before_append` construit un événement d’issue signé valide, puis ajoute au même en-tête `Content-Length` un champ `cOnTeNt-LeNgTh` contradictoire (longueur annoncée + 1). Le receiver répond HTTP 400 `request_malformed`, non réessayable, et les octets de l’inbox restent inchangés. Une requête signée corrigée, avec le même ID et un seul `Content-Length`, est acceptée HTTP 202 à la séquence 1; le journal contient alors seulement l’événement corrigé.

Commande : `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_case_variant_conflicting_content_lengths_before_append -- --exact`. Résultat : **1 réussi, 0 échec, 122 filtrés**, test 0,07 s après compilation locale. Le test ne parle qu’au receiver local sur loopback; aucun endpoint externe n’est utilisé.

Ce test prouve que le parser courant compte les champs de framing sans tenir compte de la casse et refuse l’ambiguïté avant d’acquitter ou de réserver la livraison. Il ne démontre pas le comportement d’un reverse proxy placé devant le receiver.

## Vérifications

- Test processus ciblé ci-dessus : **exit 0**.
- Rustfmt, `git diff --check`, garde Cargo et vérification append-only : réussis; résultats ci-joints.

## Limites

Une seule variante de casse et une requête synthétique. Pas de proxy, capture réseau, désynchronisation multi-parser, Forgejo live, LLM/AgentOS ou benchmark. Aucun changement runtime, coût, accès élargi, secret réel ou changement de licence. La mission reste ouverte.
