# CONTINUATION-282 — refus processus d’une livraison modifiée après réception

Date : 27 octobre 2026  
Base Git déclarée par le runner : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`  
Scope : vrai binaire local `forgejo-mission-local`, Bubblewrap avec network/PID namespaces, checkout monté en lecture seule et `.runtime` tmpfs jetable.

## Résultat

Le scénario ingère un événement `issues` avec HMAC synthétique et attestation Ed25519, puis change `issue.title` vers un autre texte valide et recalcule le hash de journal non cléé. Le contrat de livraison et l’issue 41 restent valides; `read_journaled_webhook_event` accepte la ligne et `EventJournal::replay` valide sa chaîne reconstruite. Le test lance ensuite le binaire réel avec la bonne clé publique de confiance et un chemin de credential absent.

Le processus retourne `host_journal_attestation_invalid`; il ne lit donc pas le credential manquant, ne crée pas de journal de mission et ne modifie pas l’inbox. Le binaire est exécuté dans un namespace réseau isolé sans fixture API : un accès Forgejo ne peut pas réussir dans ce test. Le runner n’exécute que ce test pour ce lot; sa sortie comporte `running 1 test`, `... ok`, `1 passed; 0 failed; 0 ignored; 5 filtered out`.

## Commande et environnement

Commande :

```text
python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only launched_cli_rejects_semantically_valid_rehashed_tampering_before_credential
```

Le runner a rapporté Rust `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, Linux aarch64 (kernel `7.0.0-1019-nvidia-aarch64`). Sa copie source isolée : SHA-256 `e543085bbe8431d664ef2ef6168bfc7b84d06b704227a88ac2ba60e4efc7b8ad`; Cargo.lock `459fef3749a42c989521901dcde61cb26e12ed4206c5d722e149426c2d3a5439`; runner `1dad7cabbf4c5699c6a7e2f051dfbf6c126edd4f7322196fd6348a3dad57222e`; cache de registre isolé `1f73c6a919948c0af44acac0604f8bf4f6ff4888379605e02d3976f1baea6451`.

Les lignes de commande, versions et sortie du test sont dans `commands.txt`, `toolchain.txt` et `runner-output.txt`. Les sources examinées et hashes sont dans `source-sha256.txt`.

## Portée et limites

Cette preuve porte sur le chemin `forgejo-mission-local` et la politique de clé fournie au processus dans ce runner. Le credential est une entrée synthétique absente, pas un token réel. Elle n’établit pas l’isolement d’une installation de production, la sécurité contre un processus de même UID/root, la protection contre replay/rollback d’un ancien journal entier, l’authenticité sémantique du payload signé, une instance Forgejo live ou un runtime LLM/AgentOS. Une signature démontre provenance/intégrité relativement à la clé configurée, pas correction.

Le code runner `--only` et le résultat sont vérifiables localement; les manifestes append-only des niveaux local, worktree et projet sont actualisés dans `SHA256SUMS`.
