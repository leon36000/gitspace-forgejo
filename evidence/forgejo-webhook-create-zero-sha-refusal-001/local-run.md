# FORGEJO-WEBHOOK-CREATE-ZERO-SHA-REFUSAL-001

## Résultat

Le validateur des webhooks Forgejo `create` acceptait comme commit une chaîne de 40 zéros (et la logique générale acceptait aussi la forme de 64 zéros). La livraison pouvait être journalisée avec HTTP 202, puis réconciliée comme branche présente après une lecture API, alors que l'objet nul ne désigne pas un commit de branche.

Le contrôle ajoute `valid_nonzero_commit_sha` et l'applique uniquement à `create.sha`. Les règles de `push.before`/`push.after` gardent `valid_commit_sha` : l'OID nul est valide dans ces champs pour les transitions de création ou suppression de branche. Les tests établissent que `create` avec SHA nul est refusé avant l'ajout durable (HTTP 400, journal vide), que la réconciliation rejette les variantes 40/64 zéros avant toute lecture API, et que les tests de cycle push existants restent verts.

## Source et provenance

L'examen source ciblé porte sur Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Dans `services/webhook/notifier.go::(*webhookNotifier).CreateRef`, lignes 770-783, `CreatePayload.Sha` reçoit `refID` et `RefType` reçoit le type de la ref. Le fichier source est inchangé dans le scratch local à ce commit; blob, empreinte et limite de l'examen sont consignés dans [upstream-source-reference.txt](upstream-source-reference.txt). Forgejo n'a pas été modifié ou copié.

## TDD et vérifications

Avant le changement du validateur, les deux tests ont échoué pour le défaut visé : le reconciler retournait un état `Ok` à la place de `InvalidEnvelope`; le handler acceptait le payload signé avec HTTP 202 au lieu de 400. Après correction, `cargo test --locked --offline --all-targets -- --test-threads=1` passe avec 548 tests réussis, 0 échec, 76 ignorés sur 34 groupes. Le journal complet est [rust-all-targets.log](rust-all-targets.log). `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.

## Portée et limites

La preuve produit est une validation Rust locale utilisant une inbox HMAC temporaire et une API fixture. Les rapports Forgejo natifs antérieurs ont observé des SHA non nuls sur des créations de branches réelles (`forgejo-native-branch-create-001/002`); ce lot n'a pas démarré Forgejo et ne prouve pas comment tous les producteurs/configurations émettent des payloads. Le rejet n'ajoute pas d'authentification au-delà du secret HMAC existant et ne remplace pas la lecture canonique de l'état distant. Aucun parcours LLM, AgentOS, RAG/mémoire, benchmark apparié ou coût n'est démontré par ce lot. Le projet reste incomplet.

Le `SHA256SUMS` local vérifie 8 artefacts; les index globaux append-only vérifient les empreintes courantes de 14 chemins du produit et de 14 chemins de l'index parent. Le vérificateur confirme que les préfixes antérieurs de ces deux index n'ont pas été modifiés.
