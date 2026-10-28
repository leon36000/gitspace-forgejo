# MISSION-012-CONTROL-PLANE-MCP-ACCESS-001 — refus d'accès via les outils agent

## Changement et cause

Le test du vrai processus MCP a tenté de lire le journal webhook, le journal de mission, la configuration du host et le reçu source de webhook, puis d'écrire dans le journal webhook et le journal de mission. Les lectures étaient refusées, mais les deux écritures étaient classées `tool_input_invalid`. La cause était dans `CandidateToolHost::preflight_workspace_write` : il aplatissait toutes les variantes de `ToolRefusal`, contrairement au chemin de lecture et à l'invocation non journalisée.

Le préflight distingue maintenant `PathNotAllowed` (`tool_path_denied`) des arguments mal formés (`tool_input_invalid`). Les refus se font avant l'intent durable du writer; la forme MCP, les capacités accordées et les règles de chemins n'ont pas changé.

## Parcours vérifié

Le test crée un vrai événement webhook synthétique validé par HMAC, avec un marqueur privé dans le titre d'issue, puis prépare le reçu de source lié à cet événement pour le host. Six appels MCP hostiles tentent de lire `../webhook-inbox.jsonl`, `../mission-execution.jsonl`, `../host-config.json` et `../admission-source.json`, et d'écrire dans l'inbox et le journal. Les six appels retournent `tool_path_denied` et laissent six reçus de refus.

Le marqueur webhook n'apparaît pas en sortie MCP; les octets de l'inbox et de la source candidate restent identiques; le journal de mission conserve son préfixe puis se relit comme une chaîne de hash valide. Le host termine avec refus attendu de finalisation (`baseline_not_verified`), car ce test ne lance pas l'acceptation. Il n'exécute ni `cargo_test` ni le code candidat.

Le relevé du premier échec est dans [`red-observation.txt`](red-observation.txt). Il note explicitement que la sortie complète du run rouge n'a pas été capturée séparément.

## Vérifications

Avec Rust/Cargo 1.95.0, Linux aarch64 et Python 3.12.3 :

- le scénario de processus MCP ciblé passe 1/1;
- le test unitaire hôte de refus/receipt passe 1/1;
- `cargo test --offline --locked --test mission_012_mcp_lifecycle` passe 26 tests, 0 échec, 1 ignoré par Cargo;
- le garde Cargo et ses tests passent 7/7; les métadonnées montrent `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`;
- `cargo fmt --all -- --check` et `git diff --check` passent.

Les commandes, sorties, codes de retour, versions et empreintes sont dans ce dossier. Le manifeste local vérifie les preuves; le manifeste global append-only reçoit les hashes du lot.

## Frontière et limites

Cette preuve exerce la surface MCP du host réel avec un inbox synthétique local. Elle établit que les appels modèle exposés par ce host ne peuvent pas lire ni remplacer les fichiers de contrôle par les chemins présentés. Elle ne protège pas contre un autre processus local disposant du même UID et d'un accès filesystem direct : l'inbox 0600 et le DTO d'admission ne constituent pas une signature contre un pair de même UID. Le sandbox d'acceptation isole séparément l'exécution candidate; ce run ne l'a pas exercée. Aucun modèle, AgentOS, Forgejo live, coût, publication ou changement de production n'est impliqué.

## Test ciblé du sandbox de vérification

Pour exercer la frontière d'exécution avec le runner réel, le test d'unité `host::tests::verifier_cannot_read_host_control_file_or_write_candidate_snapshot` construit une fixture de projet corrigée et exécute `run_sandbox_test` via le binaire `mission-012-verify-local`. Deux tests ajoutés à l'acceptance candidate essaient de lire un marqueur sous le `/tmp` hôte et d'écraser un fichier déjà présent dans `tests/`. Le premier est masqué par le tmpfs `/tmp`; le second est refusé par le montage en lecture seule de `/candidate`. Les deux critères fonctionnels de `clamp_to_range` passent aussi. Résultat : code 0, 4 tests candidat réussis, fichier de contrôle et fichiers candidats inchangés; sortie brute dans [`sandbox-isolation.log`](sandbox-isolation.log), code de retour dans [`sandbox-isolation.exit`](sandbox-isolation.exit).

Ce test prouve uniquement ces propriétés avec Bubblewrap 0.9.0 et systemd-run 255 sur le Linux local. Il ne démontre pas une absence d'évasion, la séparation d'un pair de même UID, le traitement d'un noyau hostile ni la protection de tout chemin sensible. Le premier run MCP rouge et son correctif de classification restent décrits plus haut.

## Versions et limites des métadonnées

Toolchain locale au 25 octobre 2026 : `rustc 1.95.0`, `cargo 1.95.0`, Python 3.12.3, Git 2.43.0, Bubblewrap 0.9.0, systemd-run 255, Linux aarch64. Les sorties de commande et les empreintes de sources sont dans `tool-versions.txt` et `source-sha256.txt`.

La relecture des métadonnées de paquet est incluse dans le log Cargo de ce dossier : aucune licence produit choisie (`license=null`, `license_file=null`), publication désactivée (`publish=[]`), droits de redistribution non évalués. Aucun service de modèle ni interface AgentOS/Forge Code/RAG/mémoire n'a été appelé.
