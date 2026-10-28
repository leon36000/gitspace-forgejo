# WEBHOOK-MISSION-ADMISSION-003 — composition receiver vers admission locale

Date d’exécution : 2026-10-23 UTC (2026-10-22, heure de Montréal). Commit HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; branche `local/WEBHOOK-005-canonical-links`. Le dépôt de travail contient de nombreux changements locaux antérieurs; ce lot n’a ni réinitialisé ni synchronisé le dépôt.

## Parcours observé

Le test `tests::receiver_process_journal_reconciles_through_local_http_adapter` dans `src/bin/forgejo-reconcile-local.rs` lance le binaire `forgejo-webhook-local` en processus réel sur loopback, avec un secret HMAC synthétique créé en mode `0600`. Il envoie deux livraisons valides, un doublon, puis un corps différent signé sous le même ID (409 `delivery_conflict`). Après arrêt SIGTERM et redémarrage sur le même journal, le rejeu d’origine est toujours identifié comme doublon; le journal contient exactement deux entrées.

Le test lit ces entrées et appelle `LocalLoopbackForgejoApi` avec un jeton synthétique. Le client HTTP de l’adaptateur lit une fixture loopback : le payload webhook dit `closed`, la réponse de current state dit `open`, et les deux livraisons partagent un GET. L’admission utilise séparément `FixtureMissionAdmissionSource`, une source mémoire de test pour branche, exigence et état d’issue; elle ne simule pas un adaptateur Forgejo live.

La première relecture d’admission de la fixture indique que l’issue est fermée; l’opération échoue avec `mission_issue_closed` et le journal de mission reste vide. Quand la même source fixture passe à `open`, l’admission crée une allocation et un claim. Une seconde livraison avec un nouvel ID mais le même binding récupère le même mission ID et la même génération, retourne `AlreadyClaimed` et n’ajoute pas de claim supplémentaire. Le test vérifie aussi les trois lectures d’issue et les deux lectures d’exigence prévues, les quatre requêtes GET, le chemin attendu et l’en-tête d’authentification synthétique.

## Vérifications reproductibles

- Le build explicite du récepteur passe dans un répertoire de cible neuf : `CARGO_TARGET_DIR=/tmp/webhook-mission-admission-003 cargo build --offline --locked --bin forgejo-webhook-local`.
- Le contrat ciblé passe : `CARGO_TARGET_DIR=/tmp/webhook-mission-admission-003 cargo test --offline --locked --bin forgejo-reconcile-local tests::receiver_process_journal_reconciles_through_local_http_adapter -- --exact --nocapture` (1 réussi, 0 échec).
- Vérification de régression : un changement temporaire qui court-circuite le contrôle d’état d’issue fait échouer ce test sur l’attente `mission_issue_closed`; le fichier source a ensuite été restauré. Son SHA-256 de restauration est `45ec1f6344ba26d0ca769b499b713613ddce28cc7450c85c935f52e67995e3e2`. Voir `webhook-mission-admission-003-red.log`.
- Suite hors ligne `cargo test --locked --offline --all-targets` avec le même `CARGO_TARGET_DIR` : 26 exécutables, 253 tests réussis, 0 échec, 7 ignorés. Clippy all-targets passe avec trois allowances pour les lints préexistantes documentées; le build release all-targets passe. Journaux : `webhook-mission-admission-003-all-targets.log`, `webhook-mission-admission-003-clippy.log` et `webhook-mission-admission-003-release-build.log`.
- Les cinq tests du contrôle de licence Cargo passent. Readback `cargo metadata --locked --offline --no-deps` : `license=null`, `license_file=null`, publication désactivée (`publish=[]`); aucune licence produit n’est choisie. Voir `cargo-license-state-002-check.json` et `cargo-license-state-002-tests.log`.

## Couverture et limites

Test de processus local réel pour le receiver, test du client HTTP vers une fixture loopback, et test d’intégration d’admission composé dans le même harnais. La partie Forgejo API et la source branche/exigence/issue sont synthétiques; l’admission ne constitue pas un nouvel appel à Forgejo. La sonde courante n’a pas trouvé d’instance Forgejo accessible sur `127.0.0.1:3130`. Aucun modèle, agent LLM, runtime réel, PR, vérificateur indépendant, décision ou fusion n’est exercé ici. Ce lot ne mesure pas l’efficacité produit et ne démontre pas de gain général.

Le travail prépare le raccord des frontières déjà présentes dans le dépôt; il ne règle pas l’authentification de production, la provenance des critères ou l’autorité de fusion. Tout statut de vérification et toute fusion restent hors de ce parcours. Une preuve HMAC ou un hash décrit l’intégrité/provenance des octets observés, pas la correction du code.

Revue indépendante en lecture seule : aucun défaut logique majeur ni fuite de processus; elle confirme la limite des fixtures. Détails : `webhook-mission-admission-003-review.md`.
