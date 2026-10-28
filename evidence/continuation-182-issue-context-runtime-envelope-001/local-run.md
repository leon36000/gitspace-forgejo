# CONTINUATION-182 — issue courant dans le DTO d’admission

Date : 27 octobre 2026. Worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, sale avant et après ce lot. Aucun reset, commit, publication ou appel de modèle.

## Objectif et acceptation

Faire passer le texte d’issue courant au contrat d’exécution sans le mélanger aux exigences versionnées, sans le lire depuis une ancienne delivery et sans dépasser le budget déclaré. L’admission doit épingler cette lecture dans la demande sérialisée; une nouvelle valeur avec la même génération doit provoquer un conflit d’idempotence.

## Changement observé dans la source

- `LocalLoopbackForgejoSource::read_issue` parse maintenant un contexte facultatif depuis le GET courant. Le champ n’existe que si `title`, `body` et `updated_at` sont présents et valides; `body: null` est distinct d’un champ absent.
- `IssueSnapshotV1` transporte `work_item_content`; `prepare_admission` le prend du même snapshot que celui dont il vérifie le numéro, l’URL canonique et l’état `open`.
- `AgentExecutionRequestV1.untrusted_work_item` est un champ facultatif non fusionné avec `context.text` ou `requirements_json`. Il porte le scope et la route API, le numéro, `updated_at`, le titre, le type du corps, `source=current_forgejo_issue_api`, `trust=untrusted_user_content` et un SHA-256 de sa structure.
- Validation : source/type, scope/route, numéro égal au `source_issue_number` des exigences protégées, timestamp Forgejo valide, titre ≤1 024 octets, corps texte ≤4 096 octets, empreinte concordante et somme de l’estimation JSON/4 avec les tokens du contexte ≤ budget. Un contenu absent ou trop gros est omis; si le budget ne permet pas de l’inclure, l’admission poursuit sans ce champ.
- Le journal de mission hash la demande complète. Le test rejoue la même allocation avec un `updated_at` et un texte modifiés; le journal retourne `execution_idempotency_conflict`, sans traiter cette demande comme un replay exact.
- Le champ est `Option` avec `serde(default, skip_serializing_if)`: une ancienne requête v1 sans propriété se décode comme `None`. Le schéma demeure v1 pour cette extension facultative.

L’empreinte lie les champs sérialisés et permet de détecter une altération de la structure. Elle ne certifie pas que Forgejo a fourni le contenu, qu’il est exact ou sûr. Le code appelant, son scope et sa lecture API font partie de la base de confiance locale.

## Vérifications exécutées

```text
cargo fmt --all -- --check                         exit 0
git diff --check                                   exit 0
cargo test --locked --offline --all-targets --quiet exit 0
```

La suite comptabilise 36 exécutables de test, 655 tests réussis, 0 échec et 121 ignorés. Sortie brute : [`rust-all-targets.log`](rust-all-targets.log). Les tests nouveaux couvrent parsing issue courant, URL de route, corps null/absent/trop gros, timestamp invalide, intégrité, mauvaise issue, plafond de tokens, omission selon budget, protection du document/hash de requirements, compatibilité sans champ et conflit sous clé d’idempotence réutilisée.

`cargo metadata --locked --offline --format-version 1 --no-deps` confirme encore `license=null`, `license_file=null`, `publish=[]`; le contrôle local renvoie `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. Aucun dépendance n’a été ajoutée.

Sources et empreintes du snapshot courant : [`source-hashes.txt`](source-hashes.txt). Les hashes couvrent les fichiers entiers d’un worktree préexistant fortement modifié; ils n’isolent pas un diff par rapport à la base.

## Portée et limites

Ce lot est testé aux niveaux parser, demande d’admission et DTO Rust. Il n’a pas lancé un serveur Forgejo dans ce run, n’a pas appelé `AgentRuntimeV1::execute`, et n’a pas exécuté AgentOS/LLM. Le trait expose maintenant le champ à une implémentation, mais aucune consommation du texte par un modèle n’est prouvée. L’issue ne devient ni une exigence, ni une autorisation, ni une preuve.

`updated_at` et le digest empêchent un rejeu silencieux si une nouvelle admission lit un contenu différent sous la même génération; ils ne rendent pas atomiques la lecture et l’exécution et ne révoquent pas une demande déjà remise après une édition ultérieure. L’estimation JSON/4 n’est pas une tokenisation réelle ni une mesure de facturation. L’omission liée au budget ne distingue pas, dans le DTO, le contenu inconnu de celui omis; le consommateur doit traiter l’absence comme inconnue. Aucun gain général n’est mesuré. Mission incomplète.
