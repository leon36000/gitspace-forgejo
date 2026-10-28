# CONTINUATION-134 — tentative corrigée après substitution d’en-tête webhook

## Scénario et résultat

Le test `tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_a_signed_body_with_a_mismatched_event_header_before_append` lance le vrai binaire receiver sur loopback avec un secret HMAC synthétique. Le corps signé est une notification `issues` valide. Avec le même corps et le même identifiant de livraison, l’en-tête non signé `pull_request` est refusé HTTP 400 (`event_payload_invalid`); le journal reste vide. Le test renvoie ensuite le même corps, la même signature et le même identifiant avec l’en-tête `issues` correct : le receiver répond HTTP 202, `duplicate=false`, séquence 1. Le journal contient exactement un événement `forgejo.webhook.v1.issues` lié à cet ID. Cela vérifie au niveau processus qu’une validation rejetée ne réserve pas l’identifiant et n’empêche pas une reprise corrigée.

Vérification ciblée : 1 test réussi, 0 échec, 0 ignoré, 112 filtres non exécutés. Le code runtime n’a pas changé; le diff est un test processus ciblé.

## Métadonnées de licence Cargo

Readback frais du garde : `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, export SPDX `NOASSERTION`. Les neuf tests du garde passent. `Cargo.toml` garde `publish=false`, sans `license` ni `license-file`; aucune licence ou autorisation de redistribution n’est sélectionnée. Aucun changement Cargo n’était nécessaire pour lever l’ambiguïté : le contrat est déjà explicite et vérifié.

## Vérifications et portée

`cargo fmt --all -- --check`, `git diff --check`, les neuf tests du garde et le readback hors ligne passent. Les commandes, versions, logs et SHA-256 des sources sont joints.

Le receiver est un vrai processus local; le corps et la clé HMAC sont des fixtures. Ce scénario ne prouve pas qu’un serveur Forgejo live émet ces octets, ne teste pas toutes les combinaisons d’en-têtes ni un attaquant concurrent, et ne lit pas ensuite l’événement par le CLI de réconciliation. La lecture canonique HTTP est couverte par d’autres preuves citées dans `docs/checkpoint.md`; ce lot ne les rejoue pas. Aucun LLM, AgentOS, production, dépense ou service externe n’est impliqué. Le projet reste incomplet.
