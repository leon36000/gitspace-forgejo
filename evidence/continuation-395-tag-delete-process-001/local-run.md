# CONTINUATION-395 — refus du tag `delete` dans le processus receiver

Date : 28 octobre 2026. Worktree local `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout est déjà fortement modifié; aucun changement antérieur n’a été réinitialisé, remplacé, indexé ou commité.

## But et changement

CONT-394 classait correctement les tags `create` comme non pris en charge, mais son processus receiver ne couvrait pas encore le cas `delete`, alors que `delete` est un événement annoncé. Ce lot ajoute `actual_receiver_classifies_tag_delete_and_non_string_ref_type_without_journaling` au test de processus et à l’allowlist `scripts/run_webhook_reconcile_cli_e2e.py`. Aucun code d’exécution, schéma de capacité ou règle de prise en charge n’a changé.

Le scénario démarre le vrai binaire receiver compilé depuis ce worktree, crée d’abord une inbox vide mode 0600 et lit `GET /forgejo/capabilities`. Il vérifie que le contrat `delete` dit `ref_type is branch`, puis envoie deux requêtes HMAC-signées :

- Payload Forgejo valide de suppression d’un tag : HTTP 422 `unsupported_event`, non réessayable, `details.field_path=payload.ref_type`, action de refus attendue, et inbox inchangée.
- Payload `delete` avec `ref_type: 1` : HTTP 400 `event_payload_invalid`, non réessayable, `details.field_path=payload.ref_type`, conseil exact demandant `branch`, et inbox toujours inchangée.

Après les deux refus, le replay de l’inbox demeure vide. Le test confirme que le tag valide et le champ mal typé gardent des classes d’erreur différentes et ne réservent aucune livraison.

## Commandes et résultat

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_classifies_tag_delete_and_non_string_ref_type_without_journaling
```

Le runner utilise Bubblewrap avec `--unshare-net --unshare-pid`, une fixture HTTP sur loopback, un secret HMAC synthétique et un port éphémère. La vérification finale dans `receiver-tag-delete-and-malformed-ref-type-final.log` donne `1 passed; 0 failed; 0 ignored; 188 filtered out`; le premier run est aussi conservé dans `receiver-tag-delete-and-malformed-ref-type.log`. Le test de processus est ignoré dans les exécutions Cargo ordinaires et sélectionné explicitement ici.

Vérifications finales :

- `cargo fmt --all -- --check` passe (`rustfmt-check-final.log`). Le premier contrôle a seulement relevé le retour à la ligne de l’assertion ajoutée; l’assertion a été formatée et le contrôle final repasse (`rustfmt-check.log` conserve le premier résultat).
- `git diff --check` passe (`diff-check-final.log`).
- `python3 scripts/verify_cargo_license_state.py` passe (`license-state-final.log`) : `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`.
- Suite `--all-targets` non rejouée : ce lot ajoute seulement un scénario processus; aucun code d’exécution n’a changé. Le dernier résultat complet antérieur appartient à CONT-394 et n’est pas revendiqué comme une exécution de ce lot.

Outils : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, `bubblewrap 0.9.0`; le runner construit hors ligne avec `--offline --locked`.

## Identité des sources

Les empreintes ci-dessous identifient les fichiers entiers après ce lot, dans un worktree toujours sale; elles ne constituent pas des empreintes de patch isolé :

```text
64b8b5fdc651f22a515c09e6e7d19bfaa774b5baf975029a694d84d2418fc575  tests/webhook_receiver_process_e2e.rs
df73eadbc217bf496c64a0ca639f40caa2825e6630c80603bd6ccb8ca1d8ddb3  scripts/run_webhook_reconcile_cli_e2e.py
```

Les manifestes append-only locaux, worktree et projet, avec les préfixes capturés, figurent dans `SHA256SUMS`, `manifest-prefix-before.json`, `manifest-check.txt` et `global-manifest-check.txt`.

## Portée et limites

Le test exerce le receiver réel du produit, mais seulement avec un client HTTP fixture local et un HMAC synthétique; il ne prouve ni l’émission native Forgejo d’un webhook de suppression de tag ni la prise en charge du cycle de vie des tags. Il ne teste pas la réconciliation, la fusion, l’AgentOS/LLM ou un gain comparatif. Aucune messagerie, connexion GitLab, donnée privée, dépense, production ou service externe n’a été utilisée dans ce lot. La mission reste incomplète.
