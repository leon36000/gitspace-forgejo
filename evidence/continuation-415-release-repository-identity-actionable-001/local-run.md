# CONTINUATION-415 — erreur de champ pour l’identité du dépôt d’un webhook

Date : 28 octobre 2026. Révision examinée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; checkout préexistant fortement modifié. Tous les changements de ce lot sont restés dans le worktree local.

## Lacune et changement

À l’ingress, `validate_event_payload` faisait disparaître la cause précise des erreurs de `parse_resource_ref` derrière `InvalidEventPayload`. Le nouveau précontrôle valide `repository.full_name` ou `run.repository.full_name` avant append : valeur présente, paire owner/repository valide et cohérence avec les métadonnées facultatives `name` et `owner.login`. Une identité absente, mal formée ou contradictoire devient `InvalidEventPayloadField` sur le chemin correct. Le traitement de replay/réconciliation et ses erreurs n’ont pas été modifiés.

La réponse HTTP fournit une consigne propre à chaque chemin : demander au sender de confiance un corps nouvellement signé avec l’identité cohérente, consulter `/forgejo/capabilities` puis redélivrer sous le même delivery ID. Elle ne reflète ni le payload privé ni sa valeur fautive; le corps refusé n’est pas journalisé.

## Vérification des comportements

Les tests processus du vrai receiver couvrent Release et `workflow_run`. Pour les deux, ils envoient successivement une identité absente, une chaîne sans `/`, puis une paire syntaxiquement valide qui contredit `name`/`owner.login`. Chaque livraison est refusée en HTTP 400 non réessayable avec le chemin exact (`payload.repository.full_name` ou `payload.run.repository.full_name`), l’action ciblée, aucun marqueur privé et un journal vide. Le même ID corrigé est ensuite admis en séquence 1; le replay contient seulement le corps corrigé.

Le test rouge, relancé sur une copie de source qui rétablit le mapping générique antérieur, montre `field_path=null` pour Release et `workflow_run`. L’extension rouge avec identité contradictoire distingue la deuxième cause (`ResourceMismatch`) du cas `InvalidRepository`; le précontrôle final couvre les deux avant d’appeler le parseur commun.

Résultats frais :

- Deux scénarios processus, chacun sous Bubblewrap `--unshare-net --unshare-pid` : **2/2 réussis**; journaux `process-final-green.log`.
- Cibles `webhook_contract`, `webhook_handler_contract`, `webhook_reconcile_contract`, `webhook_http_contract` : **147 réussis, 0 échec**; `ingress-contracts-final.log`.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : **757 réussis, 0 échec, 199 ignorés, 38 groupes**; `all-targets-final-3.log`. Les tests ignorés ne font pas partie de ce run.
- `webhook_quarantine_cli` : **7/7 réussis** après un ajustement de son helper : avec `CARGO_TARGET_DIR=/dev/shm`, `bwrap --dev /dev` masquait le chemin de l’exécutable. Le helper copie maintenant le binaire dans le dossier scratch déjà monté et le rend non modifiable. Le premier all-targets et le retry après `cargo build --bins` gardés dans les logs échouaient sur ce problème d’environnement; le run final passe.
- `cargo fmt --all -- --check`, `git diff --check`, parsing AST du runner et `python3 scripts/verify_cargo_license_state.py` : réussis.
- `cargo metadata --offline --locked --no-deps --format-version 1` : `license=null`, `license_file=null`, `publish=[]`; la décision reste `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`.

Commandes, versions, sortie Cargo, garde, métadonnées et empreintes source sont conservées dans ce dossier. Les compilations Cargo ont utilisé `/dev/shm`; les fichiers temporaires des tests ordinaires ont utilisé `/tmp` puis ont été gérés par les tests.

## Portée et limites

Les signatures HMAC et les événements sont synthétiques; le receiver est le vrai binaire local et les requêtes HTTP restent sur loopback dans les espaces réseau isolés. Aucun webhook émis par Forgejo, instance Forgejo live, agent LLM, AgentOS, coût externe ou service distant n’a été utilisé. Le résultat valide ce refus à l’ingress pour les familles Release et workflow testées, pas chaque variante de chaque événement ni une frontière de sécurité contre root/noyau.

Suite : éprouver au processus l’identité du dépôt de base d’un événement PR et distinguer cette identité de celle du dépôt source/fork autorisé.
