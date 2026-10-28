# CONTINUATION-303 — processus receiver et retry corrigé du même webhook

Date: 2026-10-27. Worktree produit basé sur f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, fortement modifié avant ce lot. Rust/Cargo 1.95.0, Linux/aarch64. Ce lot ajoute une preuve de processus au correctif parser/handler CONT-302; il n’ajoute pas de code de production.

## Scénario et résultat

Le test actual_receiver_rejects_conflicting_issue_root_number_before_append_and_accepts_corrected_retry lance le binaire forgejo-webhook-local via CARGO_BIN_EXE dans un processus enfant. Le harnais réserve un port 127.0.0.1 éphémère, sonde le receiver, écrit un secret synthétique avec permissions 0600 et utilise un journal temporaire.

1. Il signe number=42 avec issue.number=41 et l’envoie sous l’en-tête issues.
2. Le vrai processus retourne HTTP 400, accepted=false, event_payload_invalid, retryable=false et une action indiquant que le corps n’a pas été journalisé. Le replay de l’inbox est vide.
3. Le test corrige uniquement le numéro racine à 41, signe à nouveau et réutilise le même delivery ID.
4. Le même processus accepte HTTP 202, accepted=true, duplicate=false, sequence=1. Après arrêt propre, le readback trouve exactement une ligne où le numéro racine et issue.number valent tous deux 41.

Cela démontre que le refus ne réserve pas le delivery ID et que la redélivraison corrigée est durable dans la fixture locale.

## Références et vérification

- Test: tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_conflicting_issue_root_number_before_append_and_accepts_corrected_retry.
- Harnais port: start_ready_receiver lie 127.0.0.1:0 puis lance le binaire réel. Secret synthétique 0600: write_secret.
- Commande: cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_rejects_conflicting_issue_root_number_before_append_and_accepts_corrected_retry -- --exact.
- Résultat: 1 passed, 0 failed, 0 ignored; 165 tests filtrés; 0,07 s. Sortie complète: process-test.log.
- SHA-256 binaire receiver local: a88cbaaf306753a0d16b3d3093b49fafbe5633e13e3c5f6eef61b7a5bdd2e21d. Binaire du test: 2de81d5bc5cc61d68974a11e58d37af106ce736798bbd7de93eacee5a880b848.
- cargo fmt --all -- --check, git diff --check et scripts/verify_cargo_license_state.py: réussis. Cargo reste license=null, license_file=null, publish=[], redistribution_rights=not_assessed, NOASSERTION.
- La régression qui a motivé le code est documentée avec le rouge/vert parser dans CONTINUATION-302.

## Limites

Le binaire receiver est réel, mais le serveur, le secret, le corps signé et le client sont des fixtures du test. Aucun Forgejo live, reconciler, mission, LLM/AgentOS, PR, vérificateur indépendant ou merge n’est exercé dans ce cas. Il s’agit d’une seule exécution ciblée du processus, pas de la suite E2E complète ni d’une mesure de productivité. Aucun code Forgejo/GitSpace n’a été copié, aucun service externe ou coût n’a été utilisé.
