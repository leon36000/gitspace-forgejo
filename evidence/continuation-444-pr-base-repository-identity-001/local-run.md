# CONTINUATION-444 — identité complète du dépôt base d’une PR

Date : 28 octobre 2026. Worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` avec modifications locales préexistantes; aucun commit créé.

## Constat et changement

`PullRequestSnapshotV1::from_forgejo_response` liait l’URL, le numéro et `base.repo.full_name` au dépôt attendu, mais ignorait `base.repo.name` et `base.repo.owner.login`. Une réponse synthétique pouvait donc déclarer simultanément le nom canonique cible et des métadonnées de dépôt contradictoires. Le parseur vérifie maintenant les trois champs contre le propriétaire et le dépôt configurés; un objet `base.repo` absent ou mal formé est inconclusif/refusé.

Le test unitaire couvre `name` et `owner.login` divergents, les champs manquants, et conserve le refus existant de `full_name` divergent. Le test processus configure une PR dont le `owner.login` de la base est étranger, alors que `name`, `full_name`, l’URL et la route restent cohérents. Le receiver accepte la delivery HMAC synthétique; le CLI renvoie `inconclusive / mission_source_invalid_response` avant les revues, l’allocation ou le handoff. Après correction de la réponse, le retry du même delivery réclame la génération 1, écrit les deux événements attendus et conserve une seule delivery dans l’inbox. Un test positif distinct garde l’acceptation d’une PR dont la base est le dépôt cible et dont la branche head vient d’un fork.

Les fixtures de tests Rust et Python ont été complétées pour représenter la réponse v17 attendue. Le premier unittest Python a échoué avant le GET de PR parce que sa réponse Issue avait omis `repository.owner`, `repository.name` et `repository.full_name`, exigés par le parseur d’issue déjà strict. La fixture a été corrigée; aucun runtime n’a été assoupli. Les métadonnées ajoutées correspondent au DTO Forgejo source.

## Source et compatibilité examinées

Le checkout local du dépôt officiel Forgejo v17.0.0 est épinglé au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les fichiers amont ciblés examinés sont propres dans ce checkout; d’autres fichiers du scratch contiennent des POC antérieurs. `modules/structs/pull.go::PRBranchInfo.Repository` utilise `*Repository`; `modules/structs/repo.go::Repository` expose `Owner`, `Name`, `FullName`; `modules/structs/user.go::User.UserName` porte `json:"login"`. `services/convert/pull.go::ToAPIPullRequest` appelle `ToRepo` pour `Base.Repository`; `services/convert/repository.go::innerToRepo` remplit Owner, Name et FullName. Les tests portent sur le schéma source ciblé, pas sur un serveur Forgejo exécuté. Aucun code Forgejo n’a été copié.

## Vérifications

Les commandes exactes, résultats, empreintes et versions sont dans `commands.txt`, `test-results.txt` et `environment.txt`.

- Unité `forgejo_merge_preflight` : 14/14.
- Processus receiver + vrai `forgejo-mission-local`, Bubblewrap avec espaces réseau et PID isolés : 1/1; la commande est exécutée avec `--exact --ignored --nocapture` par le runner dédié.
- Processus préflight positif dépôt-base cible / fork-head : 1/1.
- `forgejo_reconcile_capabilities_e2e` : 5 réussis, 2 ignorés explicitement car ils réservent le port fixe 3130.
- Unittest Python du processus d’admission v3 : 1/1 après correction de sa fixture.
- Compilation du test merge post-crash : réussie avec `--no-run`.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : code de sortie 0 sur l’arbre corrigé. La sortie multi-cible a été tronquée par l’outil; aucun total agrégé n’est revendiqué.
- `cargo fmt --all -- --check`, `git diff --check`, `py_compile` et `scripts/verify_cargo_license_state.py` : réussis après les changements.
- L’audit local de liens Markdown a vérifié 1 641 liens des cinq documents canoniques, sans cible manquante.

## Limites

L’E2E utilise une API HTTP loopback synthétique, une seule réponse contradictoire (`n=1`), et ne prouve pas l’authenticité d’un serveur ou d’une réponse. Les trois champs sont des identifiants textuels, sans vérification d’ID numérique. Les GET de Forgejo ne forment pas un snapshot atomique; une modification peut arriver après la lecture. Ce lot ne démontre ni Forgejo live, ni LLM/AgentOS, ni la valeur comparative ou une frontière de privilèges multi-tenant. La compatibilité est vérifiée pour le code v17.0.0 ciblé; les versions différentes restent à tester.

## Suite

Auditer les autres lecteurs de PR employés par la réconciliation et le readback post-fusion, puis tester une seule lacune supplémentaire si les appels source confirment un écart. GitLab PC4 demeure en lecture seule pour l’instant; le blocage de transfert et l’alerte de secrets CI sont documentés dans le checkpoint.
