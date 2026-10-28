# CONTINUATION-391 — conseil de correction pour `create` sans `sha`

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Défaut et correction

Au receiver-process, un événement `create` de branche sans `sha` était correctement refusé avant l’écriture durable avec HTTP 400 et `details.field_path=payload.sha`, mais l’action disait seulement d’inspecter le contrat. Le test processus `actual_receiver_refuses_missing_create_sha_with_actionable_guidance` reproduit l’action générique; le mapping HTTP donne maintenant le format actuellement accepté par le parseur : un hash commit hexadécimal non nul de 40 ou 64 caractères. L’action demande une redelivery avec le même delivery ID et rappelle que le corps rejeté n’a pas été journalisé.

Un probe distinct a d’abord envoyé `review.content` comme objet. Le receiver l’a accepté en HTTP 202 et l’a journalisé; ce champ est validé par la projection de notification du réconciliateur, pas par la validation d’ingress, comme le vérifie déjà `malformed_review_notification_content_is_rejected_before_forgejo_read`. Ce probe de conception est conservé dans `review-content-ingress-probe.log`; il ne constitue pas une régression d’ingress et n’a pas motivé de changement.

## Résultats

Test-first : `receiver-sha-red.log` échoue sur l’action générique après avoir confirmé le refus et le chemin. Après correction, le vrai `forgejo-webhook-local` sous Bubblewrap passe 1/1 (185 filtrés). L’inbox reste vide, le refus est non réessayable et le conseil nomme le format requis. `cargo fmt --all -- --check`, `git diff --check`, l’analyse AST du runner et la garde Cargo passent. Métadonnées inchangées : `license=null`, `license_file=null`, `publish=[]`, SPDX `NOASSERTION`, droits de redistribution non évalués. Une première vérification rustfmt a trouvé deux écarts de format du test; correction manuelle, puis vérification finale réussie.

## Accès GitLab PC4

Le remote `pc4-gitlab` est déjà configuré vers le dépôt demandé. Les reçus réseau antérieurs montrent que les ports PC4 étaient injoignables depuis PC6; aucun test réseau n’a été répété dans ce lot. La recherche de courriels indiquée par l’utilisateur ne fournit qu’un PAT destiné à un autre périmètre et un accès initial privilégié de compte root; aucun n’a été utilisé. L’aperçu de recherche Gmail a exposé les valeurs des identifiants à l’outil; les révoquer/renouveler, sans recopier les valeurs dans ce dépôt. Aucun credential, API GitLab, `ls-remote`, staging, commit ou push utilisé. La migration reste bloquée jusqu’à une route PC6→PC4 fonctionnelle et un credential neuf limité à `root/gitspace-forgejo`.

## Portée

Cette preuve concerne le conseil d’ingress pour un champ de payload absent. Elle ne démontre ni émission native Forgejo, ni LLM/AgentOS, ni PR/fusion, ni valeur comparative. Le projet reste incomplet. Les commandes, sorties rouge/verte, état des outils et empreintes sont dans ce répertoire.
