# CONTINUATION-281 — altération sémantiquement valide d’une ligne attestée

Date : 27 octobre 2026  
Base examinée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`  
Portée : test unitaire Rust local; aucune instance Forgejo ni aucun modèle.

## Contrat vérifié

La chaîne SHA-256 du journal détecte les corruptions accidentelles, mais elle n’est pas une authentification : un écrivain capable de réécrire le journal peut recalculer cette chaîne. Le chemin privilégié doit donc vérifier l’attestation de receiver Ed25519 par une clé de confiance externe avant d’utiliser la livraison. La signature porte sur la séquence, les IDs, le type, le payload et le hash précédent. Le lecteur legacy non authentifié ne constitue pas une source d’autorité.

Le test ingère un événement d’issue no 41 avec des valeurs synthétiques, vérifie que l’attestation passe, modifie le payload en issue no 42 et recalcule le hash non cléé. Le readback legacy accepte la ligne puisque son JSON, son contrat d’événement et sa chaîne restent valides. Le readback avec la clé de confiance refuse ensuite la même ligne avec `WebhookError::Attestation(InvalidSignature)`. La modification est fonctionnellement valide comme payload d’issue; elle ne dépend pas d’un événement mal formé pour être rejetée.

L’implémentation d’attestation était déjà présente avant ce lot. Ce changement ajoute le test de falsification manquant; il ne crée pas de mécanisme de sécurité nouveau.

## Vérifications

Environnement observé : Rust `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Linux `aarch64`.

| Commande | Résultat |
|---|---|
| `cargo test --locked --offline --lib trusted_receiver_attestation_rejects_semantically_valid_rehashed_payload_tampering` | code 0; 1 réussi, 0 échoué, 22 filtrés |
| `cargo test --locked --offline --lib` | code 0; 23 réussis, 0 échoué, 0 ignoré |
| `cargo fmt --all -- --check` | code 0 |
| `git diff --check` | code 0 |

Le test utilise un HMAC et une clé de signature synthétiques, un `EventJournal` temporaire et un appel direct aux lecteurs. Il ne vérifie pas le lancement du CLI, l’ordre de lecture du credential, la politique Bubblewrap ou un serveur Forgejo. Il ne prouve pas que le contenu signé est vrai : la signature établit seulement la provenance et l’intégrité par rapport à la clé configurée. Le mode de compatibilité non authentifié reste non autoritaire; la protection contre le rollback complet d’un ancien journal signé demeure un problème distinct.

## Sources

Les versions exactes des sources de ce lot sont dans `source-sha256.txt`. Les index locaux, worktree et projet sont étendus append-only par `append_manifests.py`; le script vérifie les hashes et préserve les préfixes précédents.
