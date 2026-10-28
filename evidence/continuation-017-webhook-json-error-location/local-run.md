# CONTINUATION-017 — position d’un JSON webhook invalide

Date de travail : 25 octobre 2026. Dépôt produit : `agent-forge-core` 0.1.0, checkout de travail HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié. Sources finales, outils et commandes sont consignés dans ce dossier.

## Lacune et comportement ajouté

Le receiver authentifie d’abord les octets bruts, puis les décode en JSON strict (y compris le refus des noms de membres dupliqués). En cas de parse impossible, `WebhookError::InvalidJson` perdait `serde_json::Error::line()` et `column()`; la réponse `payload_invalid` ne permettait donc pas à l’émetteur de localiser le défaut.

Le parseur conserve maintenant ces valeurs dans `WebhookError::InvalidJsonAt { line, column }`. Le receiver maintient HTTP 400, `payload_invalid`, et `retryable=false`, puis ajoute seulement `error.details.line` et `error.details.column`. L’action demande de corriger le JSON à sa source à l’aide de la position, puis de le redélivrer avec une signature valide. Aucun octet du corps ni texte brut de parseur n’est renvoyé. `/forgejo/capabilities` décrit ce cas sous `error_contract.malformed_json`. Le corps demeure hors du journal. Un JSON décodé avec succès mais dont la racine n’est pas un objet reste rejeté par l’ancien chemin générique.

## Preuve test-first

Le nouveau test processus envoie des octets JSON invalides dont le HMAC est valide. Avant le correctif, il échoue avec `details.line = null` au lieu de 2 (`red.log`, code 101). Après l’implémentation, un premier essai échoue parce que mon calcul manuel attendait la colonne 26 alors que le parseur rapportait 25 (`column-calibration.log`, code 101). L’attente a été alignée sur la valeur observée du parseur; le test final passe avec ligne 2/colonne 25 (`green-process.log`, code 0).

Le payload contient `UNTRUSTED_SENTINEL`; la réponse ne le contient pas, reste non réessayable et la lecture du journal avant/après est identique. Un deuxième cas altère un caractère du HMAC d’un corps invalide : le receiver renvoie `401/invalid_signature` sans coordonnées, sans écho du marqueur et sans append. Des essais additionnels passent pour deux formes de membre JSON dupliqué, pour les contrats unitaires correspondant aux noms de membres littéral/échappé, et pour la découverte des capacités. Les tests dupliqués vérifient des positions positives et l’absence de l’URL sentinelle dans la réponse. Ces cas s’exécutent dans un vrai processus receiver local avec client loopback et fichier temporaire; le processus reste sous le même UID, sans namespace OS séparé.

## Vérification finale

- `cargo test --locked --offline --all-targets` : **538 réussis, 0 échec, 76 ignorés, 34 groupes**, code 0 (`all-targets.log`, `all-targets.exit`, `all-targets-summary.txt`).
- Tests ciblés verts : processus HTTP JSON invalide, signature altérée, doublons JSON, deux tests de contrat d’ingestion des noms dupliqués, et endpoint de capacités (`green-*.log`, `invalid-signature.log`).
- `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/test_cargo_license_state.py` (9/9), `python3 scripts/verify_cargo_license_state.py` et `cargo metadata --no-deps --format-version 1` passent; sorties et codes consignés.
- La métadonnée directe reste `license=null`, `license_file=null`, `publish=[]`; le garde annonce `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`. Aucun choix de licence ou dépendance nouvelle.

## Limites

Le test est un fixture de webhook local et ne vérifie pas un payload provenant d’un serveur Forgejo vivant. Il ne teste aucun LLM, AgentOS, RAG, CI, coût ou mesure comparative. L’ajout de la variante à l’enum Rust public peut exiger une mise à jour chez un consumer externe utilisant un `match` exhaustif; aucun consumer externe n’a été inspecté. La position facilite la correction, mais ne prouve ni l’identité de l’émetteur au-delà du HMAC configuré, ni la correction sémantique du payload. Projet global incomplet.

Suite prévue : comparer les scénarios de refus webhook/adaptateur déjà prouvés à la matrice d’exigences, choisir une combinaison manquante avant une nouvelle implémentation, et garder l’audit des writers Forgejo/LFS dans le scratch.
