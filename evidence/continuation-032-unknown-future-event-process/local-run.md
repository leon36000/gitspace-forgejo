# CONTINUATION-032 — refus processus d’un type webhook futur

Date : 26 octobre 2026. Le checkout local reste sur `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec les modifications préexistantes conservées.

## Lacune et vérification

`tests/webhook_reconcile_contract.rs::unsupported_notification_never_calls_forgejo_api` vérifie le refus dans la fonction de réconciliation, mais ne lance pas le binaire sur le journal. Le nouveau test `actual_reconcile_cli_process_refuses_unknown_future_event_kind_before_api_read` utilise le receiver réel pour écrire une livraison issue admise, arrête le receiver, puis transforme la ligne temporaire en `forgejo.webhook.v1.future_event` et recalcule son hash de chaîne. Le CLI réel est lancé contre ce journal sous le runner Bubblewrap allowlisté et isolé du réseau externe.

Résultat : 1 test réussi, 0 échec, 84 filtrés. Le CLI retourne `inconclusive`, `unsupported_event`, `retryable=false`, zéro tentative de lecture amont et l’action `--capabilities`. Le listener réservé sur le port Forgejo ne reçoit aucune connexion; le journal reste octet pour octet inchangé. Le scénario est autorisé explicitement par `scripts/run_webhook_reconcile_cli_e2e.py`; il demeure ignoré dans les suites génériques qui ne créent pas le namespace réseau requis.

Commande exacte et sortie complète : [commands.txt](commands.txt), [focused.log](focused.log). Vérifications d’hygiène : `cargo fmt --all -- --check`, compilation Python de l’allowlist, `git diff --check`, garde de licence et readback Cargo metadata passent. Versions et empreintes sont dans les fichiers associés.

## Portée et limites

C’est un fixture synthétique à chaîne d’intégrité locale valide. Il ne prouve pas qu’une livraison future authentique est signée, qu’un producteur Forgejo émet ce type, ni que le format de journal a une authentification cryptographique. Le receiver courant n’a pas admis ce type; aucun code de production ne change. Aucun serveur Forgejo distant, AgentOS, modèle, coût ou benchmark n’est utilisé. Le parcours LLM, la comparaison contrôlée et l’autorité serveur sur tous les writers restent non démontrés.
