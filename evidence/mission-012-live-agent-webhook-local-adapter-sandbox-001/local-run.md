# MISSION-012 sandbox-001 — adaptateur isolé, MCP stdio et PR locale

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, modifié et non commité. Ce lot ajoute un mode local explicite `--sandbox-test-adapter`; il reste réservé au `local-test-adapter` sans LLM.

## Frontière exercée

Le wrapper de confiance lit un fichier MCP mode `0600` et démarre le superviseur/hôte avant l’adaptateur. Bubblewrap démarre le processus d’adaptateur avec `--unshare-net --unshare-pid --new-session`, un environnement vidé, `/usr` et le script fixture en lecture seule, `/tmp` et `/home` privés, et sans montage du workspace, de la configuration MCP ou des reçus hôte. `Popen(close_fds=True)` n’utilise pas `pass_fds`; après le raccordement, le wrapper ferme ses copies des deux pipes. stdin/stdout transportent les trames MCP vers l’hôte de confiance; stderr transporte un événement JSON d’état vers le wrapper.

Le processus a lu le contexte admis puis appelé uniquement les quatre outils exposés par l’hôte : `mission_context`, `workspace_read_text`, `workspace_write_text` et `cargo_test`. Le wrapper a fourni un fichier canari synthétique dans le `/tmp` hôte et un listener TCP loopback actif. Le résumé retourné indique `credential_visible=false` et `network_connected=false`; après la sortie, le listener parent n’avait accepté aucune connexion. Le test de contrat reproduit le parcours par stdio avec un faux hôte MCP limité; il vérifie aussi qu’une interruption après écriture ferme le canal, finalise l’hôte et ne lance pas le test vert.

## Composition réelle locale

Commande complète dans [commands.txt](commands.txt). Le runner a capturé 86 fichiers (3 115 485 octets), archive SHA-256 `e98c3bb21ba951513b157c214c785f0e9d3a697a2f815ef4e2e8edbf239dc499`, manifeste source SHA-256 `2a93c714923153eef0f6c09c57f82d0e7c05255c358f86a7ac4b63085fad4cdb`. Forgejo local était le binaire 17.0.0+gitea-1.22.0 avec le hash vérifié `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

Les quatre scripts d’exécution (wrapper, adaptateur, runner MCP et runner composé) ont chacun le même SHA-256 actuel que dans le snapshot de cette composition; les valeurs sont consignées dans `composition_source_sha256` de `composed-run.json` et dans `source-hashes.txt`.

`composed-run.json` rapporte `merged_in_private_sandbox`. La demande webhook a été liée à l’exigence et au commit de base `991fbf6549ba4d0ee937c58ad350c857fd603a58`. L’outil a observé l’échec initial actionnable (`left: -2`, `right: 10`), puis a corrigé uniquement `src/lib.rs`; le candidat `4227d378b6f640542a470675e32add0afeb73105` a passé les deux tests d’acceptation. Le vérificateur indépendant et la revérification pré-fusion ont chacun passé sur ce même commit. La PR #2 a ensuite été fusionnée par `forgejo-reconcile-local-rust` dans le bac à sable au commit `7aaa94688199a944d705d24ec680c8ab31001eb8`; le readback webhook indique `verified_current_merged_pr`.

Le résumé de l’étape agent [composed-stage-summaries.json](composed-stage-summaries.json) lie le résultat au profil `bubblewrap_stdio_mcp`, avec les deux probes refusés, `model_invoked_during_run=false` et coût rapporté `0.0 USD`. Le plafond configuré de `0.01 USD` n’est pas une dépense. Les journaux du rouge et du vert sont [base-red-cargo-test.log](base-red-cargo-test.log) et [candidate-green-cargo-test.log](candidate-green-cargo-test.log). Les huit tests unitaires ciblés, y compris le refus avec Claude et l’interruption stdio, passent selon [unit-tests.log](unit-tests.log).

## Portée et limites

Forgejo, le dépôt, la PR et l’autorité de fusion étaient locaux et temporaires sous `/tmp`; aucune production ni publication n’a été touchée. Les identifiants Forgejo étaient des fixtures locales côté hôte; seul le canari de lecture était synthétique et non sensible. Aucun trafic externe ni appel LLM n’a eu lieu. Ce run teste l’adaptateur déterministe et une vraie composition avec le MCP host/Forgejo; le faux MCP host du test de contrat vérifie le raccordement/protocole et n’est pas le host de cette composition.

Ce résultat ne démontre pas un parcours LLM ou AgentOS, une isolation multi-tenant de production, une résistance générale au même UID, au noyau/Bubblewrap/compilateur, ni un gain comparatif. La composition 046 a exercé séparément le comportement après fusion; ce lot ne le rejoue pas. Un run réussi unique ne prouve aucune amélioration générale des exigences satisfaites, régressions échappées, interventions, délai, tokens ou coût.

Manifestes : `SHA256SUMS` couvre les artefacts de ce dossier; `global-current-entries.txt` liste les entrées courantes du manifeste global vérifiables par `sha256sum -c` depuis la racine du worktree. Les empreintes source locales sont dans [source-hashes.txt](source-hashes.txt).
