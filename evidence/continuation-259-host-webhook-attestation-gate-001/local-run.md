# CONTINUATION-259 — épingler la confiance webhook au host MCP

Date : 27 octobre 2026. Base examinée : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche locale `local/WEBHOOK-005-canonical-links`. Le worktree était déjà fortement modifié; aucune réinitialisation, suppression, publication, fusion Git ou dépense externe n’a eu lieu. Versions, commandes et empreintes sont adjacentes.

## Résultat implémenté

Le fichier privé de configuration v5 de `mission-012-mcp-host` accepte maintenant `trusted_webhook_attestation_public_keys_hex`. En mode `webhook_delivery`, cette politique doit contenir 1 à 8 clés Ed25519 publiques en hexadécimal minuscule, chacune exactement 32 octets; une clé mal formée, dupliquée ou l’absence de toute clé est refusée. En mode `direct_local`, cette liste doit rester vide. Le host relit le reçu d’admission et l’événement du journal avec `read_journaled_webhook_event_verified`, puis compare les bindings. Une signature manquante, d’une clé non approuvée ou invalide est refusée avant lease et avant outils MCP. Les clés sont configurées hors journal; aucune clé privée n’est transmise au host.

Le test processus dédié sous Bubblewrap exerce le vrai binaire `mission-012-mcp-host`: politique vide avant ouverture d’une inbox inexistante, mauvaise clé, reçu absent, reçu pointant vers un autre événement, puis source synthétique signée correcte. Les refus ne créent pas le journal de reçus; le cas valide permet l’initialisation MCP puis s’arrête comme prévu parce qu’aucun test d’acceptation n’a été exécuté. Résultat final : **1 passé, 0 échec, 0 ignoré, 30 filtrés**. Le contrat de source vérifie également en test Rust la clé correcte, la mauvaise clé et une ligne historique non signée; les tests de politique et de lecture avant ouverture passent chacun 1/1.

## Vérifications et essais bloqués

`cargo check --offline --locked --all-targets`, `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py` passent. Le runner Bubblewrap sélectionne maintenant uniquement les binaires référencés par les macros `CARGO_BIN_EXE_*` des cibles choisies et vérifie leur taille agrégée avant copie. Son ancien essai copiait un `forgejo-reconcile-local` non requis et dépassait le tmpfs de 128 MiB (`No space left on device`); l’échec initial est conservé.

Le scénario MISSION-012 plus large a ensuite exécuté le vrai receiver, l’admission et le host avec la clé attendue et a exposé le contexte issu du journal signé. Il n’a pas atteint le test rouge/vert : le `cargo_test` du host s’est arrêté avec `Failed to connect to bus: No medium found`, car l’enveloppe Bubblewrap du runner masque le bus requis par `systemd-run --user`. J’ai laissé le socket D-Bus hors du sandbox pour ne pas élargir les accès du test. Ce parcours est donc **inconclusif** pour le changement et l’acceptation; il n’est pas compté comme test réussi de bout en bout. Un premier essai du test processus ciblé a aussi détecté une fixture qui gardait le chemin d’inbox inexistant du cas précédent; après restauration du chemin réel dans le cas mauvaise clé, le rejeu ciblé passe. Tous les échecs et le rejeu sont conservés dans les logs.

## Portée et limites

La preuve établit le contrôle strict du host MCP avec une fixture signée, et le passage receiver réel → host → contexte dans le scénario plus large jusqu’au point où le sandbox de vérification bloque. Elle ne démontre pas un LLM, AgentOS, une instance Forgejo, une PR, un merge, une comparaison ou un gain. Le résumé complet et les codes de sortie sont dans `checks-summary.json` et `commands.txt`.

`forgejo-mission-local` et `forgejo-reconcile-local` utilisent encore le lecteur de compatibilité sans clé de confiance. Le CLI de mission peut donc lire le journal, contacter l’API et employer son credential avant ce gate MCP; l’attestation ne constitue pas encore une barrière commune. Reste à configurer les clés hors journal à ces consommateurs et à vérifier avant tout readback/credential, puis définir rotation, révocation, politique des lignes historiques et anti-rollback/troncature. Le projet reste incomplet.
