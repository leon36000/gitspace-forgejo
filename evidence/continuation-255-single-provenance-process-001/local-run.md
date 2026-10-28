# CONTINUATION-255 — provenance single au niveau processus

Date : 27 octobre 2026. HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree sale conservé. Ce lot n’a modifié aucun code produit.

## Résultat principal

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_gitea_alias_only_delivery`. Le runner compile hors ligne le test et les binaires Rust, puis exécute le test ignoré sous Bubblewrap `--unshare-net --unshare-pid --new-session`, avec `prlimit`. Le test passe **1/1**, 153 filtrés; sortie complète : `single-gitea-alias.log`.

La fixture signe un événement d’issue synthétique avec le secret de test; le vrai receiver accepte les alias `X-Gitea-*`, ajoute une ligne au journal, et le vrai `forgejo-reconcile-local` lit l’état courant depuis une API HTTP loopback synthétique. Le test vérifie un seul ID et `schema_version=11`, le verdict `current_state_read`, l’état canonique `open`, une lecture API, l’absence d’en-tête `Authorization`, `replay_hmac_verification=not_performed` et `source_authenticity_claim_for_selected_delivery=not_made`. Le readback ne modifie pas le journal.

## Correction de portée

Un premier scénario exécuté dans ce lot, `actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture`, passe aussi 1/1; son log est conservé dans `single-process.log`. La revue de sa source a montré qu’il lit deux IDs et valide donc le chemin batch, déjà couvert par CONT-253. Il est explicitement exclu du résultat single nouveau et ne sert pas de preuve distincte.

## Limites

Le scénario nouveau confirme le contrat single par un processus receiver→CLI réel, mais emploie un événement `X-Gitea-*` accepté par le récepteur Forgejo, pas une instance serveur Forgejo. L’API est une fixture locale; le secret est synthétique; aucun credential d’API n’est utilisé. Le run n’effectue pas de PR/merge, ne prouve pas l’authenticité du writer local, ne persiste ni ne revérifie le HMAC, et n’emploie ni AgentOS ni LLM. Aucun gain n’est mesuré. Le contrat explicite CONT-253 est maintenant exercé en processus single; les lacunes de confiance et le benchmark restent ouverts.

La source de preuve du test exact est `tests/webhook_receiver_process_e2e.rs::actual_reconcile_cli_process_reads_gitea_alias_only_delivery`; le helper `assert_reconcile_schema_version` lie le verdict single v11 aux deux valeurs de provenance. Les hashes sont dans `source-sha256.txt`; les index append-only sont vérifiés par le script associé.
