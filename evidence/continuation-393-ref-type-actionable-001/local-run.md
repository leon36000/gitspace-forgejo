# CONTINUATION-393 — action exploitable pour `ref_type` absent

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Défaut et correction

Le test receiver-process existant `actual_receiver_localizes_missing_branch_ref_type_before_durable_append` vérifie qu’un événement `create` sans `ref_type` est refusé en HTTP 400 avec `details.field_path=payload.ref_type`, sans entrée de journal. Il ne vérifiait pas le champ `error.action`, encore générique. Test-first, l’assertion précise a échoué sur cette réponse; le mapping HTTP indique maintenant que les webhooks de branche requièrent `ref_type=branch` et que les références de tag ne sont pas prises en charge par cet adaptateur. Il conseille la redelivery sous le même delivery ID et rappelle que le corps refusé n’est pas journalisé.

Aucune règle d’admission, source de vérité ou version du schéma ne change. L’action s’aligne sur le validateur courant : valeurs autres que `branch` sont refusées comme événements non pris en charge.

## Résultat

La commande ciblée passe 1/1 (186 filtrés) et conserve l’inbox vide. Le test lance le processus receiver réel et l’atteint uniquement via son adresse loopback éphémère `127.0.0.1`; il a été exécuté directement, pas sous Bubblewrap. Cargo utilise `--offline --locked`. `cargo fmt --all -- --check`, `git diff --check`, l’analyse AST du runner et la garde Cargo passent.

Les métadonnées restent `license=null`, `license_file=null`, `publish=[]`, SPDX `NOASSERTION`; aucune licence produit n’est choisie et les droits de redistribution ne sont pas évalués. Aucun changement Cargo.

## Limites

Une omission `ref_type` dans un événement `create` seulement est exercée. Les valeurs JSON non textuelles, les événements delete et les tags n’ont pas été rejoués dans ce scénario. Il ne s’agit pas d’une émission native Forgejo, d’un parcours AgentOS/LLM, d’une PR/fusion ou d’une mesure comparative. Aucun service externe, courriel ou credential n’a été utilisé.
