# CONTINUATION-168 — audit du dépôt GitSpace public courant

Date : 26 octobre 2026. Le dépôt produit examiné reste le worktree fortement modifié dont la base est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun fichier GitSpace ni Forgejo n’a été copié dans ce worktree; l’audit GitSpace est dans le scratch `/tmp/gitspace-audit-dcde1142cf76`.

## Identité et licence

`git ls-remote https://github.com/leon36000/GitSpace.git HEAD refs/heads/main` et l’API publique GitHub identifient `main` au commit `dcde1142cf76996a6ba0074cb7eab9d3c870c561`. Le tree Git de ce commit est `760dfeca4fcc8f43c4799bea37eed37f1ea9f618`, avec 217 fichiers suivis. Sa date de commit est `2026-08-14T23:32:51-04:00`; aucun tag de version n’est établi par ce contrôle. Le shallow clone est détaché sur ce SHA et `git status --porcelain` est vide après l’examen.

L’API GitHub déclare `license: null`; le tree n’a pas de chemin nommé `LICENSE`, `LICENCE`, `COPYING` ou `NOTICE`; le README à cette ref affiche `UNKNOWN` et dit qu’aucune licence n’est choisie implicitement. Cela ne détermine pas le droit d’auteur ni ne donne un droit d’utilisation ou de redistribution. Aucun code GitSpace n’est copié/adopté. Le dépôt est public, mais son README décrit le projet comme un bootstrap en PR brouillon et affirme qu’aucune forge GitSpace complète n’est encore implémentée.

## Contenu exécutables, schémas et tests observés

Le code source contredit une lecture littérale de « aucun code » : sept crates Rust version `0.1.0`, édition 2024, sont membres de la workspace, toutes `publish = false` :

- `gs-canonical-json` calcule des représentations canoniques/digests; `gs-eval-ir` définit des modèles et valide les JSON contre les schémas `schemas/v1/`.
- `gs-cas` et `gs-event-journal` fournissent un CAS fichier et un journal local : l’append vérifie l’identité/séquence, lie les événements par digest de chaîne et appelle `sync_all`; la lecture reconstruit la projection et vérifie les contenus.
- `gs-verdict::issue_verdict` calcule un verdict déterministe à partir d’un `VerdictInput`; l’indépendance d’une vérification ne vient donc pas de cette seule fonction.
- `gs-local-runner::LocalRunner` interprète une liste d’opérations `Read`, `Write`, `Delay` sur fixtures dans un workspace de run, avec capability prefixes et oracle. C’est un interpréteur de scénario déterministe; il ne lance ni processus d’agent, ni conteneur/sandbox OS, ni LLM.
- `gs-foundry-cli` expose `run` et `replay` pour les scénarios `pass|fail|timeout|policy|infra`. Le replay ouvre le store en lecture seule.
- Les tests Rust de workspace couvrent contrats, intégrité, adversarial, cycle de vie, délais/budgets, replay et substitutions. Des schémas JSON v1 et des contrats de parité Python existent également.

Ces éléments prouvent du code exécutable d’une Evaluation Foundry déterministe, pas une forge, un protocole AgentOS, un runtime LLM, un système général de vérification indépendante, des pulls requests/fusions Forgejo ou un service de reprise durable en production. Aucun audit exhaustif des 217 fichiers, des workflows distants ni des dépendances/licences transitives n’est revendiqué.

## Vérification locale

Toolchain déclarée : Rust/Cargo `1.97.1` via `rust-toolchain.toml`. Hôte : Linux `7.0.0-1019-nvidia aarch64`; Bubblewrap `0.9.0`. Aucun `build.rs` n’est présent dans le tree. Une recherche statique dans les crates/tests n’a pas trouvé d’ouverture réseau; les tests appellent le binaire de la workspace et créent des fixtures temporaires.

Le premier `cargo test --offline` n’a pas compilé : `serde_json_canonicalizer` manquait au cache. Le lockfile ne référence que `crates.io`; `cargo fetch --locked` a donc rempli un nouveau cache sous `/tmp/gitspace-audit-cargo-home`. Ce fetch a utilisé le service public crates.io uniquement, avec un environnement processus vidé, HOME masqué, source montée en lecture seule et sans clé/credential; aucune source ou donnée privée n’a été transmise.

Après ce fetch, un seul run a exécuté `cargo test --quiet --offline --locked --workspace --all-targets` dans Bubblewrap : code de sortie 0 en 15,744 s; les 34 blocs Cargo affichés totalisent 107 tests réussis, zéro échec. La source était en lecture seule, le réseau et PID namespaces étaient isolés, HOME et `/tmp` étaient temporaires, et la cible était `/tmp/gitspace-audit-target`, hors du produit. `test-observed-summary.txt` conserve le résumé retranscrit de la sortie; ce n’est pas un dump brut byte-for-byte. La commande de reproduction et l’isolation sont dans [commands.txt](commands.txt).

Les hashes de 12 fichiers source/schéma/test ont été relus par `sha256sum -c` et passent tous; l’état Git après le test reste propre. Les versions et le readback sont dans [source-hash-check.txt](source-hash-check.txt). Les tests Python, clippy/fmt, GitHub Actions, une instance Forgejo, l’adaptateur AgentOS/RAG/mémoire et tout LLM n’ont pas été exécutés. Le run établit seulement que les cibles Rust du snapshot compilent et que leurs tests présents passent dans cet environnement isolé. Cela ne vaut pas validation de sécurité, conformité de licence ou intégration produit.

## Archives et limites d’examen

La recherche de noms exacts `GitSpace-main.zip` et `forgejo-forgejo (2).zip` sous `/home/pc6/Documents/Codex` n’a retourné aucun chemin. La recherche est limitée à cette racine et ne prouve pas une absence sur tout le système; aucun hash d’archive sans historique n’est donc disponible ici. Le snapshot GitSpace public utilisé a un SHA commit exact. L’examen de code s’est concentré sur les sept manifests Rust, le runner, le journal, le verdict, la CLI, le schéma de manifeste et les tests concernés; il n’est pas une revue complète des mécanismes de sécurité ni une analyse juridique.

Forgejo demeure séparé : le code officiel v17.0.0 déjà audité est épinglé au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le FAQ officiel indique GPL v3+ pour v9 et ultérieurs. Aucun code Forgejo n’est copié. Références primaires datées : [GitSpace à la ref examinée](https://github.com/leon36000/GitSpace/tree/dcde1142cf76996a6ba0074cb7eab9d3c870c561), [README à cette ref](https://github.com/leon36000/GitSpace/blob/dcde1142cf76996a6ba0074cb7eab9d3c870c561/README.md), [workspace Cargo](https://github.com/leon36000/GitSpace/blob/dcde1142cf76996a6ba0074cb7eab9d3c870c561/Cargo.toml), [FAQ licence Forgejo](https://forgejo.org/faq/).

## Résumé de résultat

| État | Résultat |
|---|---|
| Conçu dans GitSpace | Schémas et types d’évaluation; opérations, journaux, verdicts et artefacts de la Foundry |
| Implémenté au snapshot | 7 crates Rust C0 et CLI locale, avec tests présents |
| Exécuté ici | Tests Rust `workspace --all-targets`, 107 réussis/0 échoué |
| Non démontré | Agent LLM/AgentOS, forge ou PR/merge, vérificateur d’autorité distincte, comportement de service distant, gain de productivité |
| Droits | Licence toujours inconnue; pas de réutilisation dans le produit |
