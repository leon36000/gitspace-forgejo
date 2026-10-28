# CONTINUATION-013 — refus du membre `review` au vrai receiver HTTP

Date locale : 2026-10-25  
Révision examinée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (arbre de travail modifié; le test et les documents de ce lot sont non commités).  
Source amont invoquée : aucune dans ce lot. Le comportement du correctif CONTINUATION-012 est testé sans exécuter Forgejo.

## Contrat testé

`tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_signed_issue_review_member_before_append` lance le binaire compilé `forgejo-webhook-local` avec un environnement vidé. Le helper crée un répertoire temporaire privé, un secret `0600`, réserve une adresse `127.0.0.1` sur port éphémère, puis attend la disponibilité du vrai processus. Le test envoie quatre corps JSON signés par HMAC sous l’événement `issues`; chacun contient une propriété inattendue `review`, de forme `null`, booléen, texte ou tableau.

Pour chaque livraison, il exige HTTP `400`, le code structuré `event_payload_invalid`, `retryable=false` et les mêmes octets d’inbox qu’avant l’envoi. Après arrêt du receiver, la relecture du journal doit retourner zéro événement.

## Vérification

Commande exécutée depuis le worktree :

```text
cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_rejects_signed_issue_review_member_before_append -- --exact --nocapture
```

Résultat : exit 0; `1 passed`, `0 failed`, `0 ignored`, `75 filtered out`; test en 0,09 s. Le journal complet est `receiver-process-test.log`. Le premier run était déjà vert, car CONTINUATION-012 avait corrigé le contrôle de production auparavant; ce lot ne revendique pas de reproduction rouge/verte.

Outils : Rust `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`. Plateforme : Linux `7.0.0-1019-nvidia`, `aarch64`. Voir `tool-versions.txt` et `source-sha256.txt`.

## Portée et limites

Le test exerce le vrai serveur HTTP dans un processus fils, avec secret et inbox jetables; les connexions sont locales sur loopback. Il n’est pas lancé dans un namespace réseau ou utilisateur et le processus garde le même UID. Aucun Forgejo, Docker, modèle, API externe, AgentOS, credential réel ou service de production n’a été utilisé. Le test prouve les refus HTTP et la non-persistance de ces payloads; il ne prouve pas un parcours de réconciliation aval, un merge, un confinement OS, ni un gain comparatif. Le bug de forme et les tests d’ingestion/réconciliation restent documentés dans CONTINUATION-012.
