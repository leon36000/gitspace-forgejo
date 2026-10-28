# CONTINUATION-257 — sonde d’attestation Ed25519 à vérification publique

Date : 27 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun changement au runtime, à `Cargo.toml` ou au `Cargo.lock` du produit. Le projet d’essai, son propre lockfile, les reçus synthétiques et les sorties se trouvent uniquement dans ce dossier de preuve. Les paquets publics épinglés ont été récupérés depuis crates.io; aucune donnée privée ni clé opérationnelle n’a été envoyée. Le checkpoint capturé avant ce lot est conservé dans `checkpoint-before-cont257.md` et retrouve le SHA-256 historique référencé par CONT-256.

## Lacune observée

`[src/webhook.rs::ingest_forgejo_delivery](../../src/webhook.rs#L987)` vérifie le HMAC sur le corps brut avant de décoder et valider l’événement, puis journalise un envelope qui ne contient ni signature d’origine ni attestation du receiver. `[src/webhook.rs::read_journaled_webhook_event](../../src/webhook.rs#L1063)` vérifie la chaîne et l’enveloppe, pas l’identité du writer. `[src/lib.rs::event_hash](../../src/lib.rs#L1497)` est un SHA-256 de champs de ligne; un writer local qui peut réécrire et recalculer la chaîne peut donc produire une ligne cohérente sans produire une nouvelle signature Forgejo valide. Cela correspond aux limites déjà documentées en CONT-253/256.

Le HMAC partagé ne peut pas être remis au vérificateur sans lui donner aussi la capacité de fabriquer des HMAC. La séparation proposée est asymétrique : receiver hôte conserve une clé privée distincte et signe après validation HMAC; le reconciler ne reçoit qu’une clé publique épinglée par une politique hôte extérieure au journal.

## Essai reproductible

Le reçu de la sonde est un JSON déterministe avec domaine/version, fournisseur, ID de livraison, nom d’événement et SHA-256 du corps brut. Deux tests utilisent uniquement une graine fixe explicitement réservée aux tests :

- `receiver_signature_verifies_with_public_key_only` : `ed25519-dalek` signe et vérifie avec sa clé publique; `ring` vérifie la même signature avec la même clé publique.
- `wrong_key_and_changed_receipt_are_rejected` : une autre clé, un ID modifié et un digest de corps modifié font échouer la vérification.

Résultat final : `cargo test --locked --offline` réussit 2/2; `cargo fmt -- --check` et `cargo clippy --locked --offline --all-targets -- -D warnings` passent. `cargo tree` et Cargo metadata sont conservés; les licences sont disponibles pour les 49 paquets combinés de l’essai (zéro `license` manquant). Les fermetures séparées sont de 24 paquets avec `ed25519-dalek` 3.0.0 et 19 avec `ring` 0.17.14, candidat inclus.

## Comparaison ciblée des candidats

| Candidat | Version et licence du paquet | Intégration / maintenance constatée | Coût et limite pour ce besoin |
|---|---|---|---|
| `ed25519-dalek` | 3.0.0, BSD-3-Clause, MSRV Rust 1.85 | API Rust typée `SigningKey`/`VerifyingKey`, vérification et signature Ed25519; version publiée le 6 juillet 2026. Le dépôt séparé est archivé et renvoie vers le monorepo `curve25519-dalek`, qui contient toujours le sous-projet Ed25519 et des changements actifs. | Pure Rust; fermeture de 24 paquets dans la sonde. Rust 1.95 local satisfait son MSRV. Retenir ce candidat pour le prochain essai d’intégration, sous réserve d’examen des licences transitives et de la gestion des clés. |
| `ring` | 0.17.14, Apache-2.0 AND ISC, MSRV Rust 1.66 | API Ed25519 fonctionnelle, vérifiée en interopérabilité; release 0.17.14 publiée le 11 mars 2025. | Fermeture de 19 paquets dans la sonde, mais construction incluant `cc` et des sources natives/assemblage. Plus petite fermeture; version stable observée plus ancienne. Pas retenu pour le premier essai, sans verdict général de sécurité. |

La sonde construit les deux alternatives ensemble en 3,5 s une première fois sur cette machine; ce temps ne compare donc pas leur coût de compilation. Aucun fournisseur facturable ni service payant n’a été utilisé. Le compte de paquets est un proxy limité de maintenance, pas un coût total en heures ou dollars. Une intégration de production doit mesurer build, taille binaire, mises à jour et coût de maintenance sur l’environnement cible.

Sources primaires consultées le 27 octobre 2026 : [API et version `ed25519-dalek` 3.0.0](https://docs.rs/crate/ed25519-dalek/3.0.0), [dépôt courant `curve25519-dalek`](https://github.com/dalek-cryptography/curve25519-dalek), [ancienne adresse archivée et déplacée](https://github.com/dalek-cryptography/ed25519-dalek), [API et release `ring` 0.17.14](https://docs.rs/crate/ring/0.17.14), [signatures Ed25519 `ring`](https://docs.rs/ring/0.17.14/ring/signature/index.html). Les licences exactes et les fermetures de dépendances de cette sonde sont aussi produites localement par `cargo metadata` puis dérivées dans `dependency-licenses.json` et `candidate-closures.json`.

## Décision et limites

La primitive Ed25519 permet de vérifier sans partager la clé de signature. Le candidat favori pour une prochaine intégration est `ed25519-dalek` 3.0.0, mais il n’est pas ajouté aux dépendances produit et le protocole de clés n’est pas encore adopté. La sonde ne contient pas de serveur Forgejo, ne calcule pas de HMAC, ne signe pas une vraie livraison et ne lit/écrit pas le journal produit.

Pour l’intégration, signer sous le verrou d’append la séquence finale, l’ID d’événement, l’idempotency key, le kind, le payload canonique, le précédent hash, l’ID de livraison et le digest du corps; conserver la signature avec la ligne; vérifier contre une clé publique épinglée hors du journal. Une signature prouve seulement que la clé privée correspondante a signé ces octets. Il reste à vérifier la frontière receiver/agent, à définir rotation/révocation et migration des événements sans signature, et à concevoir un ancrage séparé si le rollback/tronquage du journal doit être détecté. Les signatures ne remplacent ni la vérification du HMAC à l’ingestion ni le readback Forgejo courant.

Le garde produit reste `license=null`, `license_file=null`, `publish=[]`, `NOASSERTION`; le droit de redistribution du produit n’est pas évalué. Aucun LLM/AgentOS, instance Forgejo, PR/merge ou gain comparatif n’a été testé dans ce lot.
